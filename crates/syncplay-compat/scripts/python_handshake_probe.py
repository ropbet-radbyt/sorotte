#!/usr/bin/env python3
"""Stateful Python protocol probe used by syncplay-rs compatibility tests.

The probe intentionally mirrors key legacy Syncplay server message shapes
for a focused subset of protocol behavior: Hello, List, Set, State, TLS.
"""

import hashlib
import json
import math
import os
import pathlib
import re
import sys
import time

DEFAULT_CONTROLLED_ROOM_HASH_SALT = "syncplay-rs-controlled-room-v1"
DEFAULT_UPGRADE_URL = "https://syncplay.pl"
DEFAULT_OUTDATED_MOTD_TEMPLATE = (
    "You are using Syncplay {client_version} but a newer version is available from {upgrade_url}"
)
PERSISTENT_ROOMS_NOTICE = (
    "NOTICE: This server uses persistent rooms, which means that the playlist information is stored between playback sessions. If you want to create a room where information is not saved then put -temp at the end of the room name."
)
CONTROLLED_ROOM_REGEX = re.compile(r"^\+(.*):(\w{12})$")
PASSWORD_REGEX = re.compile(r"[A-Z]{2}-\d{3}-\d{3}")
SERVER_STATE_INTERVAL_SECONDS = 1.0
INITIAL_SERVER_STATE_DELAY_SECONDS = 0.1
PROTOCOL_TIMEOUT_SECONDS = 12.5
PING_MOVING_AVERAGE_WEIGHT = 0.85


def _emit_json(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")))
    sys.stdout.write("\n")
    sys.stdout.flush()


def _extract_hello_arguments(hello):
    room_name = None
    if "username" in hello and isinstance(hello["username"], str):
        username = hello["username"].strip()
    else:
        username = None

    room = hello["room"] if "room" in hello else None
    if isinstance(room, dict):
        if "name" in room and isinstance(room["name"], str):
            room_name = room["name"].strip()
        else:
            room_name = None

    version = hello["version"] if "version" in hello else None
    version = hello["realversion"] if "realversion" in hello else version
    features = hello["features"] if "features" in hello else None
    return username, room_name, version, features


def _add_legacy_root_to_sys_path(legacy_root):
    root = pathlib.Path(legacy_root)
    if root.is_dir():
        root_str = str(root)
        if root_str not in sys.path:
            sys.path.insert(0, root_str)


def _load_syncplay_version(legacy_root):
    _add_legacy_root_to_sys_path(legacy_root)
    try:
        import syncplay  # type: ignore

        version = getattr(syncplay, "version", None)
        return version if isinstance(version, str) and version else "unknown"
    except Exception:
        return "unknown"


def _parse_numeric_version_components(version):
    if not isinstance(version, str):
        return None
    trimmed = version.strip()
    if not trimmed:
        return None

    components = []
    for part in trimmed.split("."):
        if not part or not part.isdigit():
            return None
        components.append(int(part))
    return components


def _is_client_version_outdated(client_version, server_version):
    client_components = _parse_numeric_version_components(client_version)
    server_components = _parse_numeric_version_components(server_version)
    if client_components is None or server_components is None:
        return False

    width = max(len(client_components), len(server_components))
    client_components.extend([0] * (width - len(client_components)))
    server_components.extend([0] * (width - len(server_components)))
    return client_components < server_components


def _client_version_meets_minimum(client_version, minimum_version):
    client_components = _parse_numeric_version_components(client_version)
    minimum_components = _parse_numeric_version_components(minimum_version)
    if client_components is None or minimum_components is None:
        return False

    width = max(len(client_components), len(minimum_components))
    client_components.extend([0] * (width - len(client_components)))
    minimum_components.extend([0] * (width - len(minimum_components)))
    return client_components >= minimum_components


def _legacy_client_feature_defaults(version):
    return {
        "sharedPlaylists": _client_version_meets_minimum(version, "1.4.0"),
        "chat": _client_version_meets_minimum(version, "1.5.0"),
        "featureList": False,
        "readiness": _client_version_meets_minimum(version, "1.3.0"),
        "managedRooms": _client_version_meets_minimum(version, "1.3.0"),
        "persistentRooms": False,
        "uiMode": "Unknown",
    }


def _legacy_client_features_for_version(version, advertised_features):
    if isinstance(advertised_features, dict) and advertised_features:
        return advertised_features
    return _legacy_client_feature_defaults(version)


def _render_motd_template(template, client_version, server_version):
    return (
        template.replace("{client_version}", str(client_version))
        .replace("{latest_version}", str(server_version))
        .replace("{upgrade_url}", DEFAULT_UPGRADE_URL)
    )


def _motd_for_client_version(client_version, server_version, motd_template=None):
    is_outdated = _is_client_version_outdated(client_version, server_version)
    if isinstance(motd_template, str):
        trimmed_template = motd_template.strip()
        if not trimmed_template:
            return ""
        custom_motd = _render_motd_template(
            trimmed_template, client_version, server_version
        )
        if is_outdated:
            warning_motd = _render_motd_template(
                DEFAULT_OUTDATED_MOTD_TEMPLATE, client_version, server_version
            )
            return "{}\n{}".format(warning_motd, custom_motd)
        return custom_motd

    if is_outdated:
        return _render_motd_template(
            DEFAULT_OUTDATED_MOTD_TEMPLATE, client_version, server_version
        )
    return ""


def _client_supports_persistent_rooms(features):
    if not isinstance(features, dict):
        return False
    return bool(features.get("persistentRooms", False))


def _persistent_rooms_notice_motd(
    base_motd, persistent_rooms_enabled=False, client_features=None
):
    if (not persistent_rooms_enabled) or _client_supports_persistent_rooms(client_features):
        return base_motd
    if not base_motd:
        return PERSISTENT_ROOMS_NOTICE
    return "{}\n\n{}".format(PERSISTENT_ROOMS_NOTICE, base_motd)


def _env_flag_enabled(name):
    value = os.environ.get(name)
    if not isinstance(value, str):
        return False
    return value == "1" or value.lower() in ("true", "yes")


def _env_multiline_list(name):
    value = os.environ.get(name)
    if not isinstance(value, str):
        return []
    return value.splitlines()


def _feature_ui_mode(features):
    if not isinstance(features, dict):
        return None
    mode = features.get("uiMode")
    return mode if isinstance(mode, str) else None


def _is_gui_user(features):
    mode = _feature_ui_mode(features)
    if mode is None:
        mode = "Unknown"
    if mode == "Unknown":
        mode = "GUI"
    return mode == "GUI"


class ProbeSession:
    def __init__(
        self,
        server_version,
        motd_template=None,
        persistent_rooms_enabled=False,
        permanent_rooms=None,
        tls_available=False,
    ):
        self.server_version = server_version
        self.motd_template = motd_template
        self.persistent_rooms_enabled = persistent_rooms_enabled
        self.permanent_rooms = set(permanent_rooms or [])
        self.tls_available = bool(tls_available)
        self.logged = False
        self.username = None
        self.room_name = None
        self.client_version = None
        self.client_features = None
        self.ready = False

    def _error(self, message):
        return [{"Error": {"message": message}}]

    def _server_features(self):
        return {
            "sharedPlaylists": False,
            "chat": True,
            "featureList": True,
            "readiness": True,
            "managedRooms": True,
            "persistentRooms": self.persistent_rooms_enabled,
            "uiMode": "UNKNOWN",
        }

    def _list_response(self):
        if not self.logged:
            return self._error("not-known-server-error")
        rooms = {
            self.room_name: {
                self.username: {
                    "position": 0,
                    "file": {},
                    "controller": False,
                    "isReady": self.ready,
                    "features": {},
                }
            }
        }
        if _is_gui_user(self.client_features):
            dummy_count = 0
            for room_name in sorted(self.permanent_rooms):
                if room_name == self.room_name:
                    continue
                dummy_count += 1
                rooms.setdefault(room_name, {})[" " * dummy_count] = {
                    "position": 0,
                    "file": {},
                    "controller": False,
                    "isReady": True,
                    "features": [],
                }
        return [{"List": rooms}]

    def _handle_hello(self, hello):
        if not isinstance(hello, dict):
            return self._error("hello-server-error")
        username, room_name, version, features = _extract_hello_arguments(hello)
        if not username or not room_name or not version:
            return self._error("hello-server-error")
        features = _legacy_client_features_for_version(version, features)

        self.logged = True
        self.username = username
        self.room_name = room_name
        self.client_version = version
        self.client_features = features
        self.ready = False

        motd = _motd_for_client_version(version, self.server_version, self.motd_template)
        motd = _persistent_rooms_notice_motd(
            motd, self.persistent_rooms_enabled, features
        )
        return [
            {
                "Hello": {
                    "username": username,
                    "room": {"name": room_name},
                    "version": version,
                    "realversion": self.server_version,
                    "features": self._server_features(),
                    "motd": motd,
                }
            }
        ]

    def _handle_set(self, settings):
        if not self.logged:
            return self._error("not-known-server-error")
        if not isinstance(settings, dict):
            return self._error("not-json-server-error")

        responses = []
        if "room" in settings and isinstance(settings["room"], dict):
            room_name = settings["room"].get("name")
            if isinstance(room_name, str) and room_name.strip():
                self.room_name = room_name.strip()
                responses.append({"Set": {"room": {"name": self.room_name}}})

        if "ready" in settings and isinstance(settings["ready"], dict):
            ready = settings["ready"]
            is_ready = bool(ready.get("isReady", False))
            manually_initiated = bool(ready.get("manuallyInitiated", False))
            username = ready.get("username")
            if not isinstance(username, str) or not username:
                username = self.username
            self.ready = is_ready

            ready_response = {
                "username": username,
                "isReady": is_ready,
                "manuallyInitiated": manually_initiated,
            }
            set_by = ready.get("setBy")
            if isinstance(set_by, str) and set_by:
                ready_response["setBy"] = set_by
            responses.append({"Set": {"ready": ready_response}})

        return responses

    def handle_message(self, message):
        if not isinstance(message, dict) or not message:
            return self._error("not-json-server-error")
        if len(message) != 1:
            return self._error("unknown-command-server-error")

        command, payload = next(iter(message.items()))
        if command == "Hello":
            return self._handle_hello(payload)
        if command == "List":
            return self._list_response()
        if command == "Set":
            return self._handle_set(payload)
        if command == "State":
            if not self.logged:
                return self._error("not-known-server-error")
            return []
        if command == "TLS":
            if not isinstance(payload, dict):
                return self._error("not-json-server-error")
            start_tls = payload.get("startTLS")
            if not isinstance(start_tls, str) or "send" not in start_tls:
                return []
            should_start_tls = (not self.logged) and self.tls_available
            return [{"TLS": {"startTLS": "true" if should_start_tls else "false"}}]
        if command in ("Chat", "Error"):
            return []
        return self._error("unknown-command-server-error")


class FanoutBatchProbe:
    def __init__(
        self,
        server_version,
        controlled_room_salt=DEFAULT_CONTROLLED_ROOM_HASH_SALT,
        motd_template=None,
        persistent_rooms_enabled=False,
        permanent_rooms=None,
        tls_available=False,
    ):
        self.server_version = server_version
        self.controlled_room_salt = controlled_room_salt
        self.motd_template = motd_template
        self.persistent_rooms_enabled = persistent_rooms_enabled
        self.permanent_rooms = set(permanent_rooms or [])
        self.tls_available = bool(tls_available)
        self.sessions = {}
        self.room_controllers = {}
        self.room_playlists = {}
        self.room_playback = {}
        self.client_state_counters = {}
        self.pending_client_ignoring = {}
        self.pending_client_latency = {}
        self.client_last_state_update_at = {}
        self.client_next_periodic_state_at = {}
        self.client_ping_rtt = {}
        self.client_ping_avg_rtt = {}
        self.client_ping_forward_delay = {}
        self.current_time_seconds = 0.0
        self._apply_permanent_rooms_snapshot()

    def _server_features(self):
        return {
            "isolateRooms": False,
            "readiness": True,
            "managedRooms": True,
            "persistentRooms": self.persistent_rooms_enabled,
            "chat": True,
            "featureList": True,
            "setOthersReadiness": True,
            "uiMode": "UNKNOWN",
        }

    def _error(self, client_id, message):
        return [{"client": client_id, "message": {"Error": {"message": message}}}]

    def _ensure_room_state(self, room_name):
        if room_name not in self.room_controllers:
            self.room_controllers[room_name] = set()
        if room_name not in self.room_playlists:
            self.room_playlists[room_name] = {"files": [], "index": None}
        if room_name not in self.room_playback:
            self.room_playback[room_name] = {
                "position": 0.0,
                "paused": True,
                "setBy": None,
                "updatedAt": self.current_time_seconds,
            }

    def _apply_permanent_rooms_snapshot(self):
        if not self.persistent_rooms_enabled:
            return
        for room_name in sorted(self.permanent_rooms):
            self._ensure_room_state(room_name)
            if self.room_playlists[room_name]["index"] is None:
                self.room_playlists[room_name]["index"] = 0

    def _add_controller(self, room_name, username):
        self._ensure_room_state(room_name)
        self.room_controllers[room_name].add(username)

    def _remove_controller(self, room_name, username):
        if room_name in self.room_controllers:
            self.room_controllers[room_name].discard(username)

    def _is_controller(self, room_name, username):
        return room_name in self.room_controllers and username in self.room_controllers[room_name]

    def _split_controlled_room_name(self, room_name):
        if not isinstance(room_name, str):
            return None
        match = CONTROLLED_ROOM_REGEX.match(room_name)
        if match is None:
            return None
        return match.group(1), match.group(2)

    def _is_controlled_room_name(self, room_name):
        return self._split_controlled_room_name(room_name) is not None

    def _is_valid_room_password(self, password):
        if not isinstance(password, str) or not password:
            return False
        return PASSWORD_REGEX.match(password) is not None

    def _controlled_room_hash(self, room_name, password):
        room_name_bytes = room_name.encode("utf-8")
        salt_hash = hashlib.sha256(
            self.controlled_room_salt.encode("utf-8")
        ).hexdigest().encode("utf-8")
        provisional_hash = hashlib.sha256(room_name_bytes + salt_hash).hexdigest().encode(
            "utf-8"
        )
        return hashlib.sha1(provisional_hash + salt_hash + password.encode("utf-8")).hexdigest()[
            :12
        ].upper()

    def _controlled_room_name_for(self, room_name, password):
        return f"+{room_name}:{self._controlled_room_hash(room_name, password)}"

    def _controlled_room_password_matches(self, room_name, password):
        split = self._split_controlled_room_name(room_name)
        if split is None:
            return False
        base, expected_hash = split
        return self._controlled_room_hash(base, password) == expected_hash

    def _user_can_control_playlist(self, room_name, username):
        return (not self._is_controlled_room_name(room_name)) or self._is_controller(
            room_name, username
        )

    def _all_client_ids(self, exclude=None):
        keys = sorted(self.sessions.keys())
        if exclude is None:
            return keys
        return [key for key in keys if key != exclude]

    def _to_gui_only_list_recipient_ids(self):
        recipients = []
        for client_id in self._all_client_ids():
            features = self.sessions[client_id].get("features")
            if isinstance(features, dict) and "uiMode" in features:
                recipients.append(client_id)
        return recipients

    def _find_free_username(self, username, exclude_client_id=None):
        all_names = []
        for client_id, session in self.sessions.items():
            if exclude_client_id is not None and client_id == exclude_client_id:
                continue
            all_names.append(session["username"].lower())
        if username.lower() in all_names and username.endswith("_"):
            username = username.rstrip("_") or "_"
        while username.lower() in all_names:
            username += "_"
        return username

    def _room_client_ids(self, room_name):
        return sorted(
            [
                client_id
                for client_id, session in self.sessions.items()
                if session["room"] == room_name
            ]
        )

    def _room_is_marked_temporary(self, room_name):
        if not isinstance(room_name, str):
            return False
        room_name = room_name.lower()
        return room_name.endswith("-temp") or "-temp:" in room_name

    def _room_is_persistent(self, room_name):
        return self.persistent_rooms_enabled and not self._room_is_marked_temporary(room_name)

    def _room_is_permanent(self, room_name):
        return self.persistent_rooms_enabled and room_name in self.permanent_rooms

    def _room_playback_state_at(self, room_name, now_seconds):
        self._ensure_room_state(room_name)
        state = dict(self.room_playback[room_name])
        if not state["paused"]:
            elapsed = float(now_seconds) - float(state.get("updatedAt", now_seconds))
            if math.isfinite(elapsed) and elapsed > 0:
                state["position"] = float(state["position"]) + elapsed
        state["updatedAt"] = now_seconds
        return state

    def _room_should_be_retained_when_empty(self, room_name):
        self._ensure_room_state(room_name)
        return self._room_is_persistent(room_name) and bool(
            self.room_playlists[room_name]["files"]
        )

    def _cleanup_room_if_empty(self, room_name):
        if self._room_client_ids(room_name):
            return
        if self._room_is_permanent(room_name):
            return
        if self._room_should_be_retained_when_empty(room_name):
            return
        self.room_controllers.pop(room_name, None)
        self.room_playlists.pop(room_name, None)
        self.room_playback.pop(room_name, None)

    def _empty_room_names(self):
        room_names = set()
        room_names.update(self.room_controllers.keys())
        room_names.update(self.room_playlists.keys())
        room_names.update(self.room_playback.keys())
        return sorted(
            [room_name for room_name in room_names if not self._room_client_ids(room_name)]
        )

    def _ready_message(self, username, is_ready, manually_initiated=False, set_by=None):
        ready_value = None if is_ready is None else bool(is_ready)
        ready = {
            "username": username,
            "isReady": ready_value,
            "manuallyInitiated": bool(manually_initiated),
        }
        if isinstance(set_by, str) and set_by:
            ready["setBy"] = set_by
        return {"Set": {"ready": ready}}

    def _joined_message(self, username, room_name, version, features):
        return {
            "Set": {
                "user": {
                    username: {
                        "room": {"name": room_name},
                        "event": {
                            "joined": True,
                            "version": version,
                            "features": features,
                        },
                    }
                }
            }
        }

    def _room_update_message(self, username, room_name):
        return {"Set": {"user": {username: {"room": {"name": room_name}}}}}

    def _hello_response(self, username, room_name, version, client_features):
        motd = _motd_for_client_version(version, self.server_version, self.motd_template)
        motd = _persistent_rooms_notice_motd(
            motd, self.persistent_rooms_enabled, client_features
        )
        return {
            "Hello": {
                "username": username,
                "room": {"name": room_name},
                "version": version,
                "realversion": self.server_version,
                "features": self._server_features(),
                "motd": motd,
            }
        }

    def _controller_auth_status_message(self, username, room_name, success):
        return {
            "Set": {
                "controllerAuth": {
                    "user": username,
                    "room": room_name,
                    "success": bool(success),
                }
            }
        }

    def _new_controlled_room_message(self, room_name, password):
        return {
            "Set": {
                "newControlledRoom": {
                    "roomName": room_name,
                    "password": password,
                }
            }
        }

    def _playlist_snapshot_message(self, files, set_by=None):
        playlist_change = {"files": list(files)}
        if isinstance(set_by, str) and set_by:
            playlist_change["user"] = set_by
        return {"Set": {"playlistChange": playlist_change}}

    def _playlist_index_snapshot_message(self, index, set_by=None):
        playlist_index = {"index": int(index) if isinstance(index, int) else None}
        if isinstance(set_by, str) and set_by:
            playlist_index["user"] = set_by
        return {"Set": {"playlistIndex": playlist_index}}

    def _room_sync_state_message(self, do_seek, include_ignoring):
        state = {
            "playstate": {
                "position": 0.0,
                "paused": True,
                "doSeek": bool(do_seek),
                "setBy": None,
            },
            "ping": {"latencyCalculation": self.current_time_seconds, "serverRtt": 0},
        }
        if include_ignoring:
            state["ignoringOnTheFly"] = {"server": 1}
        return {"State": state}

    def _state_update_message(self, username, playstate):
        if not isinstance(playstate, dict):
            return None

        state_playstate = {}
        position = playstate.get("position")
        if isinstance(position, (int, float)):
            state_playstate["position"] = float(position)
        paused = playstate.get("paused")
        if isinstance(paused, bool):
            state_playstate["paused"] = paused
        do_seek = playstate.get("doSeek")
        if isinstance(do_seek, bool):
            state_playstate["doSeek"] = do_seek
        state_playstate["setBy"] = username

        return {
            "State": {
                "playstate": state_playstate,
                "ping": {"latencyCalculation": self.current_time_seconds, "serverRtt": 0},
                "ignoringOnTheFly": {"server": 1},
            }
        }

    def _ack_server_ignoring_counter(self, client_id, counter):
        if not isinstance(counter, int):
            return
        if client_id not in self.client_state_counters:
            return
        if self.client_state_counters[client_id] == counter:
            self.client_state_counters[client_id] = 0

    def _next_server_ignoring_counter(self, client_id):
        current = self.client_state_counters.get(client_id, 0)
        next_counter = current + 1
        self.client_state_counters[client_id] = next_counter
        return next_counter

    def _queue_client_ignoring_counter(self, client_id, counter):
        if isinstance(counter, int):
            self.pending_client_ignoring[client_id] = counter

    def _queue_client_latency(self, client_id, client_latency):
        if isinstance(client_latency, (int, float)):
            self.pending_client_latency[client_id] = float(client_latency)

    def _take_pending_client_ignoring_counter(self, client_id):
        if client_id not in self.pending_client_ignoring:
            return None
        return self.pending_client_ignoring.pop(client_id)

    def _take_pending_client_latency(self, client_id):
        if client_id not in self.pending_client_latency:
            return None
        return self.pending_client_latency.pop(client_id)

    def _forced_state_sync_message(self, client_id, position, paused, do_seek, set_by):
        pending_client_latency = self._take_pending_client_latency(client_id)
        pending_client_ignoring = self._take_pending_client_ignoring_counter(client_id)
        state = {
            "playstate": {
                "position": float(position),
                "paused": bool(paused),
                "doSeek": bool(do_seek),
            },
            "ping": {
                "latencyCalculation": self.current_time_seconds,
                "serverRtt": self._server_rtt_for_client(client_id),
            },
            "ignoringOnTheFly": {"server": self._next_server_ignoring_counter(client_id)},
        }
        if pending_client_latency is not None:
            state["ping"]["clientLatencyCalculation"] = pending_client_latency
        if pending_client_ignoring is not None:
            state["ignoringOnTheFly"]["client"] = pending_client_ignoring
        if isinstance(set_by, str) and set_by:
            state["playstate"]["setBy"] = set_by
        return {"State": state}

    def _record_client_state_update(self, client_id):
        self.client_last_state_update_at[client_id] = self.current_time_seconds

    def _ingest_client_ping_metrics(self, client_id, ping):
        if not isinstance(ping, dict):
            return
        latency_calculation = ping.get("latencyCalculation")
        if not isinstance(latency_calculation, (int, float)):
            return
        sender_rtt = ping.get("clientRtt", 0.0)
        if not isinstance(sender_rtt, (int, float)):
            sender_rtt = 0.0
        if sender_rtt < 0:
            return

        current_rtt = float(self.current_time_seconds) - float(latency_calculation)
        if current_rtt < 0:
            return

        average_rtt = self.client_ping_avg_rtt.get(client_id, 0.0)
        if average_rtt == 0.0:
            average_rtt = current_rtt
        average_rtt = (
            average_rtt * PING_MOVING_AVERAGE_WEIGHT
            + current_rtt * (1.0 - PING_MOVING_AVERAGE_WEIGHT)
        )
        if sender_rtt < current_rtt:
            forward_delay = average_rtt / 2.0 + (current_rtt - sender_rtt)
        else:
            forward_delay = average_rtt / 2.0

        self.client_ping_rtt[client_id] = current_rtt
        self.client_ping_avg_rtt[client_id] = average_rtt
        self.client_ping_forward_delay[client_id] = forward_delay

    def _server_rtt_for_client(self, client_id):
        return self.client_ping_rtt.get(client_id, 0.0)

    def _forward_delay_for_client(self, client_id):
        return self.client_ping_forward_delay.get(client_id, 0.0)

    def _periodic_state_sync_message(self, client_id, position, paused, set_by):
        pending_client_latency = self._take_pending_client_latency(client_id)
        pending_client_ignoring = self._take_pending_client_ignoring_counter(client_id)
        if self.client_state_counters.get(client_id, 0) > 0:
            return None

        state = {
            "playstate": {
                "position": float(position),
                "paused": bool(paused),
                "doSeek": False,
            },
            "ping": {
                "latencyCalculation": self.current_time_seconds,
                "serverRtt": self._server_rtt_for_client(client_id),
            },
        }
        if pending_client_latency is not None:
            state["ping"]["clientLatencyCalculation"] = pending_client_latency
        if pending_client_ignoring is not None:
            state["ignoringOnTheFly"] = {"client": pending_client_ignoring}
        if isinstance(set_by, str) and set_by:
            state["playstate"]["setBy"] = set_by
        return {"State": state}

    def _is_client_timed_out(self, client_id, now_seconds):
        if client_id not in self.client_last_state_update_at:
            return False
        return (now_seconds - self.client_last_state_update_at[client_id]) > PROTOCOL_TIMEOUT_SECONDS

    def _remove_client_tracking(self, client_id):
        session = self.sessions.pop(client_id, None)
        if session is None:
            return None
        previous_room = session["room"]
        self._remove_controller(previous_room, session["username"])
        self.client_state_counters.pop(client_id, None)
        self.pending_client_ignoring.pop(client_id, None)
        self.pending_client_latency.pop(client_id, None)
        self.client_last_state_update_at.pop(client_id, None)
        self.client_next_periodic_state_at.pop(client_id, None)
        self.client_ping_rtt.pop(client_id, None)
        self.client_ping_avg_rtt.pop(client_id, None)
        self.client_ping_forward_delay.pop(client_id, None)
        self._cleanup_room_if_empty(previous_room)
        return session

    def _timeout_disconnect_messages(self, client_id):
        session = self._remove_client_tracking(client_id)
        if session is None:
            return []
        left_message = {
            "Set": {
                "user": {
                    session["username"]: {
                        "room": {"name": session["room"]},
                        "event": {"left": True},
                    }
                }
            }
        }
        outputs = []
        recipients = self._all_client_ids()
        recipients.append(client_id)
        for peer_id in recipients:
            outputs.append({"client": peer_id, "message": left_message})
        if self.persistent_rooms_enabled:
            for peer_id in self._to_gui_only_list_recipient_ids():
                outputs.extend(self._list_response(peer_id))
        return outputs

    def _collect_periodic_tick_for_client(self, client_id, ticked_at):
        if client_id not in self.sessions:
            return []
        session = self.sessions[client_id]
        room_name = session["room"]
        self._ensure_room_state(room_name)
        if self.room_playback[room_name]["setBy"] is None:
            fallback_set_by = self._fallback_room_set_by_username(room_name)
            if isinstance(fallback_set_by, str) and fallback_set_by:
                self.room_playback[room_name]["setBy"] = fallback_set_by
        room_state = self._room_playback_state_at(room_name, ticked_at)

        outputs = []
        periodic_state = self._periodic_state_sync_message(
            client_id,
            room_state["position"],
            room_state["paused"],
            room_state["setBy"],
        )
        if periodic_state is not None:
            outputs.append({"client": client_id, "message": periodic_state})

        if self._is_client_timed_out(client_id, ticked_at):
            outputs.extend(self._timeout_disconnect_messages(client_id))

        return outputs

    def _fallback_room_set_by_username(self, room_name):
        usernames = sorted(
            [
                session["username"]
                for session in self.sessions.values()
                if session["room"] == room_name
            ]
        )
        if not usernames:
            return None
        return usernames[0]

    def _collect_due_periodic_outputs(self):
        due_clients = sorted(
            [
                client_id
                for client_id, next_state_at in self.client_next_periodic_state_at.items()
                if next_state_at <= self.current_time_seconds
            ]
        )
        outputs = []
        for client_id in due_clients:
            next_state_at = self.client_next_periodic_state_at.get(client_id)
            if next_state_at is None:
                continue
            while next_state_at <= self.current_time_seconds:
                self.client_next_periodic_state_at[client_id] = (
                    next_state_at + SERVER_STATE_INTERVAL_SECONDS
                )
                outputs.extend(
                    self._collect_periodic_tick_for_client(client_id, next_state_at)
                )
                if client_id not in self.sessions:
                    break
                next_state_at += SERVER_STATE_INTERVAL_SECONDS
        return outputs

    def _advance_time(self, seconds):
        if not isinstance(seconds, (int, float)):
            return []
        if seconds <= 0:
            return []
        self.current_time_seconds += float(seconds)
        return self._collect_due_periodic_outputs()

    def _list_response(self, client_id):
        if client_id not in self.sessions:
            return self._error(client_id, "not-known-server-error")

        rooms = {}
        for peer_id in self._all_client_ids():
            session = self.sessions[peer_id]
            room = session["room"]
            if room not in rooms:
                rooms[room] = {}
            entry = {
                "position": 0.0,
                "file": {},
                "controller": self._is_controller(room, session["username"]),
                "isReady": session["ready"],
            }
            if session["features"] is not None:
                entry["features"] = session["features"]
            rooms[room][session["username"]] = entry
        if _is_gui_user(self.sessions[client_id].get("features")):
            dummy_count = 0
            for empty_room in self._empty_room_names():
                dummy_count += 1
                rooms.setdefault(empty_room, {})[" " * dummy_count] = {
                    "position": 0.0,
                    "file": {},
                    "controller": False,
                    "isReady": True,
                    "features": [],
                }
        return [{"client": client_id, "message": {"List": rooms}}]

    def _handle_hello(self, client_id, hello):
        if not isinstance(hello, dict):
            return self._error(client_id, "hello-server-error")
        username, room_name, version, features = _extract_hello_arguments(hello)
        if not username or not room_name or not version:
            return self._error(client_id, "hello-server-error")
        features = _legacy_client_features_for_version(version, features)

        if client_id in self.sessions:
            self._remove_client_tracking(client_id)

        username = self._find_free_username(username, exclude_client_id=client_id)
        self._ensure_room_state(room_name)
        self.sessions[client_id] = {
            "username": username,
            "room": room_name,
            "version": version,
            "features": features,
            "ready": None,
        }
        self.client_state_counters[client_id] = 0
        self.pending_client_ignoring.pop(client_id, None)
        self.pending_client_latency.pop(client_id, None)
        self._record_client_state_update(client_id)
        self.client_next_periodic_state_at[client_id] = (
            self.current_time_seconds + INITIAL_SERVER_STATE_DELAY_SECONDS
        )

        outputs = []
        joined = self._joined_message(username, room_name, version, features)
        for peer_id in self._all_client_ids(exclude=client_id):
            outputs.append({"client": peer_id, "message": joined})
        ready_message = self._ready_message(username, None, False)
        for peer_id in self._room_client_ids(room_name):
            outputs.append({"client": peer_id, "message": ready_message})

        room_playlist = self.room_playlists[room_name]
        playlist_snapshot = self._playlist_snapshot_message(
            room_playlist["files"], self.room_playback[room_name]["setBy"]
        )
        outputs.append({"client": client_id, "message": playlist_snapshot})
        playlist_index = room_playlist.get("index")
        outputs.append(
            {
                "client": client_id,
                "message": self._playlist_index_snapshot_message(
                    playlist_index, self.room_playback[room_name]["setBy"]
                ),
            }
        )

        outputs.append(
            {
                "client": client_id,
                "message": self._hello_response(
                    username, room_name, version, features
                ),
            }
        )
        if self.persistent_rooms_enabled:
            for peer_id in self._to_gui_only_list_recipient_ids():
                outputs.extend(self._list_response(peer_id))
        return outputs

    def _handle_set(self, client_id, settings):
        if client_id not in self.sessions:
            return self._error(client_id, "not-known-server-error")
        if not isinstance(settings, dict):
            return self._error(client_id, "not-json-server-error")

        outputs = []
        session = self.sessions[client_id]

        if "room" in settings and isinstance(settings["room"], dict):
            room_name = settings["room"].get("name")
            if isinstance(room_name, str) and room_name.strip():
                room_name = room_name.strip()
                if room_name != session["room"]:
                    previous_room = session["room"]
                    self._remove_controller(previous_room, session["username"])
                    self._ensure_room_state(room_name)
                    session["room"] = room_name
                    self._cleanup_room_if_empty(previous_room)
                    self.client_next_periodic_state_at[client_id] = (
                        self.current_time_seconds + SERVER_STATE_INTERVAL_SECONDS
                    )
                    room_state = self._room_playback_state_at(
                        room_name, self.current_time_seconds
                    )
                    outputs.append(
                        {
                            "client": client_id,
                            "message": self._forced_state_sync_message(
                                client_id,
                                room_state["position"],
                                room_state["paused"],
                                True,
                                room_state["setBy"],
                            ),
                        }
                    )
                    room_update = self._room_update_message(session["username"], room_name)
                    for peer_id in self._all_client_ids():
                        outputs.append({"client": peer_id, "message": room_update})
                    ready_message = self._ready_message(session["username"], session["ready"], False)
                    for peer_id in self._room_client_ids(room_name):
                        outputs.append({"client": peer_id, "message": ready_message})

                    room_playlist = self.room_playlists[room_name]
                    playlist_snapshot = self._playlist_snapshot_message(
                        room_playlist["files"], self.room_playback[room_name]["setBy"]
                    )
                    outputs.append({"client": client_id, "message": playlist_snapshot})
                    playlist_index = room_playlist.get("index")
                    outputs.append(
                        {
                            "client": client_id,
                            "message": self._playlist_index_snapshot_message(
                                playlist_index, self.room_playback[room_name]["setBy"]
                            ),
                        }
                    )
                    if self.persistent_rooms_enabled:
                        for peer_id in self._to_gui_only_list_recipient_ids():
                            outputs.extend(self._list_response(peer_id))

        if "playlistChange" in settings and isinstance(settings["playlistChange"], dict):
            files = settings["playlistChange"].get("files")
            if isinstance(files, list):
                room_name = session["room"]
                self._ensure_room_state(room_name)
                if self._user_can_control_playlist(room_name, session["username"]):
                    self.room_playlists[room_name]["files"] = list(files)
                    playlist_message = {
                        "Set": {
                            "playlistChange": {
                                "files": files,
                                "user": session["username"],
                            }
                        }
                    }
                    for peer_id in self._room_client_ids(room_name):
                        outputs.append({"client": peer_id, "message": playlist_message})
                else:
                    room_state = self.room_playlists[room_name]
                    outputs.append(
                        {
                            "client": client_id,
                            "message": {
                                "Set": {
                                    "playlistChange": {
                                        "files": room_state["files"],
                                        "user": room_name,
                                    }
                                }
                            },
                        }
                    )
                    outputs.append(
                        {
                            "client": client_id,
                            "message": self._playlist_index_snapshot_message(
                                room_state.get("index"), room_name
                            ),
                        }
                    )

        if "playlistIndex" in settings and isinstance(settings["playlistIndex"], dict):
            index = settings["playlistIndex"].get("index")
            if isinstance(index, int):
                room_name = session["room"]
                self._ensure_room_state(room_name)
                if self._user_can_control_playlist(room_name, session["username"]):
                    self.room_playlists[room_name]["index"] = int(index)
                    playlist_message = {
                        "Set": {
                            "playlistIndex": {
                                "index": int(index),
                                "user": session["username"],
                            }
                        }
                    }
                    for peer_id in self._room_client_ids(room_name):
                        outputs.append({"client": peer_id, "message": playlist_message})
                else:
                    room_state = self.room_playlists[room_name]
                    outputs.append(
                        {
                            "client": client_id,
                            "message": self._playlist_index_snapshot_message(
                                room_state.get("index"), room_name
                            ),
                        }
                    )

        if "controllerAuth" in settings and isinstance(settings["controllerAuth"], dict):
            auth = settings["controllerAuth"]
            room_to_check = auth.get("room")
            if not isinstance(room_to_check, str) or not room_to_check:
                room_to_check = session["room"]
            auth_password = auth.get("password")
            if not isinstance(auth_password, str):
                auth_password = ""

            if self._is_controlled_room_name(room_to_check):
                if not self._is_valid_room_password(auth_password):
                    auth_message = self._controller_auth_status_message(
                        session["username"], session["room"], False
                    )
                    for peer_id in self._room_client_ids(session["room"]):
                        outputs.append({"client": peer_id, "message": auth_message})
                else:
                    success = self._controlled_room_password_matches(
                        room_to_check, auth_password
                    )
                    if success:
                        self._add_controller(session["room"], session["username"])
                    auth_message = self._controller_auth_status_message(
                        session["username"], session["room"], success
                    )
                    for peer_id in self._all_client_ids():
                        outputs.append({"client": peer_id, "message": auth_message})
            elif self._is_valid_room_password(auth_password):
                controlled_room = self._controlled_room_name_for(
                    room_to_check, auth_password
                )
                outputs.append(
                    {
                        "client": client_id,
                        "message": self._new_controlled_room_message(
                            controlled_room, auth_password
                        ),
                    }
                )
            else:
                auth_message = self._controller_auth_status_message(
                    session["username"], session["room"], False
                )
                for peer_id in self._room_client_ids(session["room"]):
                    outputs.append({"client": peer_id, "message": auth_message})

        if "ready" in settings and isinstance(settings["ready"], dict):
            ready = settings["ready"]
            is_ready = ready.get("isReady", False)
            if is_ready is not None:
                is_ready = bool(is_ready)
            manually_initiated = bool(ready.get("manuallyInitiated", True))
            username = ready.get("username")
            if not isinstance(username, str) or not username:
                username = session["username"]
            if username == session["username"]:
                session["ready"] = is_ready

            ready_message = self._ready_message(
                username, is_ready, manually_initiated, ready.get("setBy")
            )
            for peer_id in self._room_client_ids(session["room"]):
                outputs.append({"client": peer_id, "message": ready_message})

        return outputs

    def _handle_state(self, client_id, state):
        if client_id not in self.sessions:
            return self._error(client_id, "not-known-server-error")
        if not isinstance(state, dict):
            return self._error(client_id, "not-json-server-error")

        session = self.sessions[client_id]
        ignore = state.get("ignoringOnTheFly")
        if isinstance(ignore, dict):
            self._ack_server_ignoring_counter(client_id, ignore.get("server"))
            self._queue_client_ignoring_counter(client_id, ignore.get("client"))

        ping = state.get("ping")
        if isinstance(ping, dict):
            self._queue_client_latency(client_id, ping.get("clientLatencyCalculation"))
            self._ingest_client_ping_metrics(client_id, ping)
        if self.client_state_counters.get(client_id, 0) > 0:
            return []
        self._record_client_state_update(client_id)

        playstate = state.get("playstate")
        if not isinstance(playstate, dict):
            return []
        room_name = session["room"]
        self._ensure_room_state(room_name)
        room_state_before = self._room_playback_state_at(room_name, self.current_time_seconds)
        can_control_room = self._user_can_control_playlist(room_name, session["username"])
        paused = playstate.get("paused")
        has_paused = isinstance(paused, bool)
        do_seek = bool(playstate.get("doSeek", False))
        pause_changed = bool(has_paused and paused != room_state_before["paused"])

        if can_control_room:
            self.room_playback[room_name]["position"] = room_state_before["position"]
            self.room_playback[room_name]["paused"] = room_state_before["paused"]
            self.room_playback[room_name]["updatedAt"] = self.current_time_seconds
            if has_paused:
                self.room_playback[room_name]["paused"] = paused
            position = playstate.get("position")
            if isinstance(position, (int, float)):
                adjusted_position = float(position)
                if not (has_paused and paused):
                    adjusted_position += self._forward_delay_for_client(client_id)
                self.room_playback[room_name]["position"] = adjusted_position
            self.room_playback[room_name]["updatedAt"] = self.current_time_seconds
            self.room_playback[room_name]["setBy"] = session["username"]

        if not do_seek and not pause_changed:
            return []

        if can_control_room:
            room_state = self._room_playback_state_at(
                room_name, self.current_time_seconds
            )
            outputs = []
            for peer_id in self._room_client_ids(room_name):
                outputs.append(
                    {
                        "client": peer_id,
                        "message": self._forced_state_sync_message(
                            peer_id,
                            room_state["position"],
                            room_state["paused"],
                            do_seek,
                            session["username"],
                        ),
                    }
                )
            return outputs

        watcher_pause_state = paused if has_paused else room_state_before["paused"]
        return [
            {
                "client": client_id,
                "message": self._forced_state_sync_message(
                    client_id,
                    room_state_before["position"],
                    watcher_pause_state,
                    False,
                    session["username"],
                ),
            },
            {
                "client": client_id,
                "message": self._forced_state_sync_message(
                    client_id,
                    room_state_before["position"],
                    room_state_before["paused"],
                    True,
                    room_state_before["setBy"],
                ),
            },
        ]

    def handle_event(self, client_id, raw_line):
        if not isinstance(client_id, str) or not client_id:
            return self._error("unknown-client", "invalid-client-id")
        if not isinstance(raw_line, str):
            return self._error(client_id, "not-json-server-error")

        try:
            message = json.loads(raw_line)
        except json.decoder.JSONDecodeError:
            return self._error(client_id, "not-json-server-error")
        if not isinstance(message, dict) or not message or len(message) != 1:
            return self._error(client_id, "unknown-command-server-error")

        command, payload = next(iter(message.items()))
        if command == "Hello":
            return self._handle_hello(client_id, payload)
        if command == "List":
            return self._list_response(client_id)
        if command == "Set":
            return self._handle_set(client_id, payload)
        if command == "State":
            return self._handle_state(client_id, payload)
        if command == "TLS":
            if not isinstance(payload, dict):
                return self._error(client_id, "not-json-server-error")
            start_tls = payload.get("startTLS")
            if not isinstance(start_tls, str) or "send" not in start_tls:
                return []
            should_start_tls = client_id not in self.sessions and self.tls_available
            return [
                {
                    "client": client_id,
                    "message": {"TLS": {"startTLS": "true" if should_start_tls else "false"}},
                }
            ]
        if command == "Chat":
            session = self.sessions.get(client_id)
            if session is None:
                return self._error(client_id, "not-known-server-error")
            if isinstance(payload, str):
                message_text = payload
            elif isinstance(payload, dict) and isinstance(payload.get("message"), str):
                message_text = payload.get("message")
            else:
                return self._error(client_id, "not-json-server-error")
            outbound = {
                "Chat": {"username": session["username"], "message": message_text}
            }
            return [
                {"client": peer_id, "message": outbound}
                for peer_id in self._room_client_ids(session["room"])
            ]
        if command == "Error":
            return []
        return self._error(client_id, "unknown-command-server-error")


def _run_single_message_mode(session):
    line = sys.stdin.readline()
    if not line:
        _emit_json({"Error": {"message": "missing-input"}})
        return 3

    try:
        message = json.loads(line)
    except json.decoder.JSONDecodeError:
        _emit_json({"Error": {"message": "not-json-server-error"}})
        return 4

    responses = session.handle_message(message)
    if not responses:
        _emit_json({"Error": {"message": "empty-response"}})
        return 7
    _emit_json(responses[0])
    return 0


def _run_batch_mode(session):
    body = sys.stdin.read()
    if not body:
        _emit_json({"Error": {"message": "missing-input"}})
        return 3

    try:
        batch = json.loads(body)
    except json.decoder.JSONDecodeError:
        _emit_json({"Error": {"message": "not-json-server-error"}})
        return 4

    inputs = batch.get("inputs") if isinstance(batch, dict) else None
    if not isinstance(inputs, list):
        _emit_json({"Error": {"message": "invalid-batch-format"}})
        return 8

    outputs = []
    for raw_line in inputs:
        if not isinstance(raw_line, str):
            outputs.append([{"Error": {"message": "not-json-server-error"}}])
            continue
        try:
            message = json.loads(raw_line)
        except json.decoder.JSONDecodeError:
            outputs.append([{"Error": {"message": "not-json-server-error"}}])
            continue
        outputs.append(session.handle_message(message))

    _emit_json({"outputs": outputs})
    return 0


def _run_fanout_batch_mode(
    server_version,
    controlled_room_salt,
    motd_template,
    persistent_rooms_enabled,
    permanent_rooms,
    tls_available,
):
    body = sys.stdin.read()
    if not body:
        _emit_json({"Error": {"message": "missing-input"}})
        return 3

    try:
        batch = json.loads(body)
    except json.decoder.JSONDecodeError:
        _emit_json({"Error": {"message": "not-json-server-error"}})
        return 4

    events = batch.get("events") if isinstance(batch, dict) else None
    if not isinstance(events, list):
        _emit_json({"Error": {"message": "invalid-fanout-batch-format"}})
        return 8

    probe = FanoutBatchProbe(
        server_version,
        controlled_room_salt,
        motd_template,
        persistent_rooms_enabled,
        permanent_rooms,
        tls_available,
    )
    outputs = []
    for event in events:
        if not isinstance(event, dict):
            outputs.append([{"client": "unknown-client", "message": {"Error": {"message": "not-json-server-error"}}}])
            continue
        step_outputs = []
        if "advanceSeconds" in event:
            advance_seconds = event.get("advanceSeconds")
            if not isinstance(advance_seconds, (int, float)):
                outputs.append([{"client": "unknown-client", "message": {"Error": {"message": "invalid-fanout-batch-format"}}}])
                continue
            step_outputs.extend(probe._advance_time(float(advance_seconds)))
        client_id = event.get("client")
        raw_line = event.get("line")
        step_outputs.extend(probe.handle_event(client_id, raw_line))
        outputs.append(step_outputs)

    _emit_json({"outputs": outputs})
    return 0


def _run_same_filename_batch_mode(legacy_root):
    body = sys.stdin.read()
    if not body:
        _emit_json({"Error": {"message": "missing-input"}})
        return 3

    try:
        batch = json.loads(body)
    except json.decoder.JSONDecodeError:
        _emit_json({"Error": {"message": "not-json-server-error"}})
        return 4

    pairs = batch.get("pairs") if isinstance(batch, dict) else None
    if not isinstance(pairs, list):
        _emit_json({"Error": {"message": "invalid-same-filename-batch-format"}})
        return 8

    _add_legacy_root_to_sys_path(legacy_root)
    try:
        from syncplay import utils as legacy_utils  # type: ignore
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-utils-import-failed",
                    "details": str(exc),
                }
            }
        )
        return 9

    outputs = []
    for pair in pairs:
        if (
            isinstance(pair, list)
            and len(pair) == 2
            and isinstance(pair[0], str)
            and isinstance(pair[1], str)
        ):
            outputs.append(bool(legacy_utils.sameFilename(pair[0], pair[1])))
        else:
            _emit_json({"Error": {"message": "invalid-same-filename-pair"}})
            return 8

    _emit_json({"outputs": outputs})
    return 0


def _run_same_filesize_batch_mode(legacy_root):
    body = sys.stdin.read()
    if not body:
        _emit_json({"Error": {"message": "missing-input"}})
        return 3

    try:
        batch = json.loads(body)
    except json.decoder.JSONDecodeError:
        _emit_json({"Error": {"message": "not-json-server-error"}})
        return 4

    pairs = batch.get("pairs") if isinstance(batch, dict) else None
    if not isinstance(pairs, list):
        _emit_json({"Error": {"message": "invalid-same-filesize-batch-format"}})
        return 8

    _add_legacy_root_to_sys_path(legacy_root)
    try:
        from syncplay import utils as legacy_utils  # type: ignore
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-utils-import-failed",
                    "details": str(exc),
                }
            }
        )
        return 9

    outputs = []
    for pair in pairs:
        if isinstance(pair, list) and len(pair) == 2:
            outputs.append(bool(legacy_utils.sameFilesize(pair[0], pair[1])))
        else:
            _emit_json({"Error": {"message": "invalid-same-filesize-pair"}})
            return 8

    _emit_json({"outputs": outputs})
    return 0


def _run_same_fileduration_batch_mode(legacy_root):
    body = sys.stdin.read()
    if not body:
        _emit_json({"Error": {"message": "missing-input"}})
        return 3

    try:
        batch = json.loads(body)
    except json.decoder.JSONDecodeError:
        _emit_json({"Error": {"message": "not-json-server-error"}})
        return 4

    pairs = batch.get("pairs") if isinstance(batch, dict) else None
    if not isinstance(pairs, list):
        _emit_json({"Error": {"message": "invalid-same-fileduration-batch-format"}})
        return 8

    show_duration_notification = batch.get("showDurationNotification")
    if show_duration_notification is not None and not isinstance(
        show_duration_notification, bool
    ):
        _emit_json(
            {"Error": {"message": "invalid-same-fileduration-show-notification-flag"}}
        )
        return 8

    different_duration_threshold = batch.get("differentDurationThreshold")
    if different_duration_threshold is not None and not isinstance(
        different_duration_threshold, (int, float)
    ):
        _emit_json(
            {"Error": {"message": "invalid-same-fileduration-threshold-value"}}
        )
        return 8

    _add_legacy_root_to_sys_path(legacy_root)
    try:
        from syncplay import utils as legacy_utils  # type: ignore
        from syncplay import constants as legacy_constants  # type: ignore
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-utils-import-failed",
                    "details": str(exc),
                }
            }
        )
        return 9

    previous_show_duration_notification = legacy_constants.SHOW_DURATION_NOTIFICATION
    previous_different_duration_threshold = legacy_constants.DIFFERENT_DURATION_THRESHOLD
    if show_duration_notification is not None:
        legacy_constants.SHOW_DURATION_NOTIFICATION = show_duration_notification
    if different_duration_threshold is not None:
        legacy_constants.DIFFERENT_DURATION_THRESHOLD = different_duration_threshold

    try:
        outputs = []
        for pair in pairs:
            if (
                isinstance(pair, list)
                and len(pair) == 2
                and isinstance(pair[0], (int, float))
                and isinstance(pair[1], (int, float))
            ):
                outputs.append(bool(legacy_utils.sameFileduration(pair[0], pair[1])))
            else:
                _emit_json({"Error": {"message": "invalid-same-fileduration-pair"}})
                return 8
    finally:
        legacy_constants.SHOW_DURATION_NOTIFICATION = previous_show_duration_notification
        legacy_constants.DIFFERENT_DURATION_THRESHOLD = (
            previous_different_duration_threshold
        )

    _emit_json({"outputs": outputs})
    return 0


def _run_client_set_file_contract_mode(legacy_root):
    _add_legacy_root_to_sys_path(legacy_root)
    try:
        from syncplay.protocols import SyncClientProtocol  # type: ignore
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-client-protocol-import-failed",
                    "details": str(exc),
                }
            }
        )
        return 9

    class RecordingClient:
        def __init__(self):
            self.calls = []

        def __getattr__(self, name):
            def _recorder(*_args, **_kwargs):
                self.calls.append(name)

            return _recorder

    def _probe_calls(file_payload):
        client = RecordingClient()
        protocol = SyncClientProtocol(client)
        protocol.handleSet({"file": file_payload})
        return client.calls

    try:
        file_payload_calls = _probe_calls(
            {"name": "movie.mkv", "duration": 95.5, "size": 123456789}
        )
        empty_payload_calls = _probe_calls({})
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-client-set-file-probe-failed",
                    "details": str(exc),
                }
            }
        )
        return 10

    _emit_json(
        {
            "filePayloadIgnored": len(file_payload_calls) == 0,
            "emptyPayloadIgnored": len(empty_payload_calls) == 0,
            "filePayloadCalls": file_payload_calls,
            "emptyPayloadCalls": empty_payload_calls,
        }
    )
    return 0


def _run_client_user_file_metadata_contract_mode(legacy_root):
    _add_legacy_root_to_sys_path(legacy_root)
    try:
        from syncplay.protocols import SyncClientProtocol  # type: ignore
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-client-protocol-import-failed",
                    "details": str(exc),
                }
            }
        )
        return 9

    class RecordingUI:
        def __getattr__(self, _name):
            def _noop(*_args, **_kwargs):
                return None

            return _noop

    class RecordingUserList:
        def __init__(self, current_username):
            self.current_username = current_username
            self._users = {}

        def _clone_file(self, file_payload):
            if file_payload is None:
                return None
            return json.loads(json.dumps(file_payload))

        def addUser(
            self,
            username,
            room,
            file_,
            noMessage=False,
            isController=None,
            isReady=None,
            features=None,
        ):
            _ = (noMessage, isController, isReady, features)
            if username == self.current_username:
                return
            self._users[username] = {"room": room, "file": self._clone_file(file_)}

        def modUser(self, username, room, file_):
            if username == self.current_username:
                return
            if username in self._users:
                self._users[username]["room"] = room
                if file_:
                    self._users[username]["file"] = self._clone_file(file_)
            else:
                self.addUser(username, room, file_)

        def removeUser(self, username):
            if username in self._users:
                self._users.pop(username)

        def clearList(self):
            self._users = {}

        def showUserList(self):
            return None

        def snapshot_files(self):
            snapshot = {}
            for username in sorted(self._users):
                file_payload = self._users[username].get("file")
                snapshot[username] = (
                    self._clone_file(file_payload)
                    if file_payload is not None
                    else None
                )
            return snapshot

    class RecordingClient:
        def __init__(self, username):
            self.ui = RecordingUI()
            self.userlist = RecordingUserList(username)

        def removeUser(self, username):
            self.userlist.removeUser(username)

        def __getattr__(self, _name):
            def _noop(*_args, **_kwargs):
                return None

            return _noop

    client = RecordingClient("interop-client")
    protocol = SyncClientProtocol(client)

    try:
        protocol.handleSet(
            {
                "user": {
                    "alice": {
                        "room": {"name": "room1"},
                        "file": {
                            "name": "**Hidden filename**",
                            "size": "15e2b0d3c338",
                            "duration": 95,
                        },
                    },
                    "bob": {
                        "room": {"name": "room1"},
                        "file": {
                            "name": "movie.mkv",
                            "size": 123456789,
                            "duration": 95.5,
                        },
                    },
                }
            }
        )
        after_set_mixed = client.userlist.snapshot_files()

        protocol.handleSet(
            {
                "user": {
                    "bob": {
                        "room": {"name": "room1"},
                        "file": {},
                    }
                }
            }
        )
        after_set_empty = client.userlist.snapshot_files()

        protocol.handleList(
            {
                "room1": {
                    "alice": {
                        "file": {
                            "name": "**Hidden filename**",
                            "size": "15e2b0d3c338",
                            "duration": 95,
                        },
                        "controller": False,
                        "isReady": True,
                        "features": {},
                    },
                    "bob": {
                        "file": {
                            "name": "movie.mkv",
                            "size": 123456789,
                            "duration": 95.5,
                        },
                        "controller": False,
                        "isReady": False,
                        "features": {},
                    },
                },
                "room2": {
                    "charlie": {
                        "file": {
                            "name": "a9858cb4803c",
                            "size": "15e2b0d3c338",
                            "duration": 95.0,
                        },
                        "controller": True,
                        "isReady": True,
                        "features": {},
                    }
                },
            }
        )
        after_list_mixed = client.userlist.snapshot_files()

        protocol.handleList(
            {
                "room1": {
                    "alice": {
                        "file": {
                            "name": "**Hidden filename**",
                            "size": "15e2b0d3c338",
                            "duration": 95,
                        },
                        "controller": False,
                        "isReady": True,
                        "features": {},
                    },
                    "bob": {
                        "file": {},
                        "controller": False,
                        "isReady": False,
                        "features": {},
                    },
                },
                "room2": {
                    "charlie": {
                        "file": {
                            "name": "a9858cb4803c",
                            "size": "15e2b0d3c338",
                            "duration": 95.0,
                        },
                        "controller": True,
                        "isReady": True,
                        "features": {},
                    }
                },
            }
        )
        after_list_clears = client.userlist.snapshot_files()
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-client-user-file-metadata-probe-failed",
                    "details": str(exc),
                }
            }
        )
        return 10

    _emit_json(
        {
            "afterSetMixed": after_set_mixed,
            "afterSetEmpty": after_set_empty,
            "afterListMixed": after_list_mixed,
            "afterListClears": after_list_clears,
        }
    )
    return 0


def _run_client_chat_send_contract_batch_mode(legacy_root):
    body = sys.stdin.read()
    if not body:
        _emit_json({"Error": {"message": "missing-input"}})
        return 3

    try:
        batch = json.loads(body)
    except json.decoder.JSONDecodeError:
        _emit_json({"Error": {"message": "not-json-server-error"}})
        return 4

    cases = batch.get("cases") if isinstance(batch, dict) else None
    if not isinstance(cases, list):
        _emit_json({"Error": {"message": "invalid-client-chat-send-contract-format"}})
        return 8

    _add_legacy_root_to_sys_path(legacy_root)
    try:
        from syncplay.client import SyncplayClient  # type: ignore
        from syncplay import constants as legacy_constants  # type: ignore
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-client-chat-import-failed",
                    "details": str(exc),
                }
            }
        )
        return 9

    class RecordingUI:
        def __init__(self):
            self.error_messages = []
            self.debug_messages = []

        def showErrorMessage(self, message, *_args, **_kwargs):
            self.error_messages.append(str(message))

        def showDebugMessage(self, message):
            self.debug_messages.append(str(message))

        def setFeatures(self, _features):
            return None

    class RecordingProtocol:
        def __init__(self, logged):
            self.logged = bool(logged)
            self.sent_messages = []

        def sendChatMessage(self, message):
            self.sent_messages.append(message)

    outputs = []
    for case in cases:
        if not isinstance(case, dict):
            _emit_json({"Error": {"message": "invalid-client-chat-send-contract-case"}})
            return 8

        message = case.get("message")
        chat_supported = case.get("chatSupported")
        protocol_logged = case.get("protocolLogged", True)
        server_version = case.get("serverVersion", "1.7.5")
        max_chat_message_length = case.get("maxChatMessageLength")
        derive_server_features = case.get("deriveServerFeatures", False)
        feature_list = case.get("featureList")

        if (
            not isinstance(message, str)
            or not isinstance(protocol_logged, bool)
            or not isinstance(server_version, str)
            or not isinstance(derive_server_features, bool)
            or (
                chat_supported is not None and not isinstance(chat_supported, bool)
            )
            or (
                max_chat_message_length is not None
                and (
                    not isinstance(max_chat_message_length, int)
                    or max_chat_message_length < 0
                )
            )
            or (
                feature_list is not None
                and not isinstance(feature_list, dict)
            )
        ):
            _emit_json({"Error": {"message": "invalid-client-chat-send-contract-case"}})
            return 8

        ui = RecordingUI()
        protocol = RecordingProtocol(protocol_logged)
        probe_client = type("ProbeClient", (), {})()
        probe_client.serverVersion = server_version
        probe_client.ui = ui
        probe_client._protocol = protocol
        probe_client._player = None
        probe_client.sendFeaturesToPlayer = lambda: None
        probe_client.addPlayerReadyCallback = lambda *_args, **_kwargs: None

        previous_max_chat_message_length = legacy_constants.MAX_CHAT_MESSAGE_LENGTH
        previous_max_username_length = legacy_constants.MAX_USERNAME_LENGTH
        previous_max_room_name_length = legacy_constants.MAX_ROOM_NAME_LENGTH
        previous_max_filename_length = legacy_constants.MAX_FILENAME_LENGTH
        previous_mpv_constants_to_send = list(
            legacy_constants.MPV_SYNCPLAYINTF_CONSTANTS_TO_SEND
        )
        try:
            if derive_server_features:
                merged_feature_list = (
                    dict(feature_list) if isinstance(feature_list, dict) else {}
                )
                if chat_supported is not None and "chat" not in merged_feature_list:
                    merged_feature_list["chat"] = chat_supported
                if (
                    max_chat_message_length is not None
                    and "maxChatMessageLength" not in merged_feature_list
                ):
                    merged_feature_list[
                        "maxChatMessageLength"
                    ] = max_chat_message_length
                SyncplayClient.checkForFeatureSupport(
                    probe_client, merged_feature_list or None
                )
            else:
                if chat_supported is None:
                    chat_supported = True
                if max_chat_message_length is None:
                    max_chat_message_length = legacy_constants.MAX_CHAT_MESSAGE_LENGTH
                probe_client.serverFeatures = {"chat": chat_supported}
                legacy_constants.MAX_CHAT_MESSAGE_LENGTH = max_chat_message_length
            SyncplayClient.sendChat(probe_client, message)
        except Exception as exc:
            _emit_json(
                {
                    "Error": {
                        "message": "legacy-client-chat-send-probe-failed",
                        "details": str(exc),
                    }
                }
            )
            return 10
        finally:
            legacy_constants.MAX_CHAT_MESSAGE_LENGTH = previous_max_chat_message_length
            legacy_constants.MAX_USERNAME_LENGTH = previous_max_username_length
            legacy_constants.MAX_ROOM_NAME_LENGTH = previous_max_room_name_length
            legacy_constants.MAX_FILENAME_LENGTH = previous_max_filename_length
            legacy_constants.MPV_SYNCPLAYINTF_CONSTANTS_TO_SEND = (
                previous_mpv_constants_to_send
            )

        outputs.append(
            {
                "sentMessages": protocol.sent_messages,
                "errorMessages": ui.error_messages,
                "debugMessages": ui.debug_messages,
            }
        )

    _emit_json({"outputs": outputs})
    return 0


def _run_privacy_file_payload_batch_mode(legacy_root):
    body = sys.stdin.read()
    if not body:
        _emit_json({"Error": {"message": "missing-input"}})
        return 3

    try:
        batch = json.loads(body)
    except json.decoder.JSONDecodeError:
        _emit_json({"Error": {"message": "not-json-server-error"}})
        return 4

    cases = batch.get("cases") if isinstance(batch, dict) else None
    if not isinstance(cases, list):
        _emit_json({"Error": {"message": "invalid-privacy-file-payload-batch-format"}})
        return 8

    _add_legacy_root_to_sys_path(legacy_root)
    try:
        from syncplay import utils as legacy_utils  # type: ignore
        from syncplay import constants as legacy_constants  # type: ignore
    except Exception as exc:
        _emit_json(
            {
                "Error": {
                    "message": "legacy-utils-import-failed",
                    "details": str(exc),
                }
            }
        )
        return 9

    outputs = []
    for case in cases:
        if not isinstance(case, dict):
            _emit_json({"Error": {"message": "invalid-privacy-file-payload-case"}})
            return 8

        file_payload = case.get("file")
        filename_privacy_mode = case.get("filenamePrivacyMode")
        filesize_privacy_mode = case.get("filesizePrivacyMode")
        if (
            not isinstance(file_payload, dict)
            or not isinstance(filename_privacy_mode, str)
            or not isinstance(filesize_privacy_mode, str)
        ):
            _emit_json({"Error": {"message": "invalid-privacy-file-payload-case"}})
            return 8

        sanitized = json.loads(json.dumps(file_payload))
        if "path" in sanitized:
            sanitized.pop("path")

        if "name" in file_payload:
            if filename_privacy_mode == legacy_constants.PRIVACY_SENDHASHED_MODE:
                sanitized["name"] = legacy_utils.hashFilename(file_payload["name"])
            elif filename_privacy_mode == legacy_constants.PRIVACY_DONTSEND_MODE:
                sanitized["name"] = legacy_constants.PRIVACY_HIDDENFILENAME
            elif filename_privacy_mode != legacy_constants.PRIVACY_SENDRAW_MODE:
                _emit_json({"Error": {"message": "invalid-filename-privacy-mode"}})
                return 8

        if "size" in file_payload:
            if filesize_privacy_mode == legacy_constants.PRIVACY_SENDHASHED_MODE:
                sanitized["size"] = legacy_utils.hashFilesize(file_payload["size"])
            elif filesize_privacy_mode == legacy_constants.PRIVACY_DONTSEND_MODE:
                sanitized["size"] = 0
            elif filesize_privacy_mode != legacy_constants.PRIVACY_SENDRAW_MODE:
                _emit_json({"Error": {"message": "invalid-filesize-privacy-mode"}})
                return 8

        outputs.append(sanitized)

    _emit_json({"outputs": outputs})
    return 0


def main():
    if len(sys.argv) < 2:
        _emit_json({"Error": {"message": "legacy-root-argument-required"}})
        return 2

    legacy_root = sys.argv[1]
    mode = "single"
    controlled_room_salt = DEFAULT_CONTROLLED_ROOM_HASH_SALT
    for argument in sys.argv[2:]:
        if argument == "--batch":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "batch"
        elif argument == "--same-filename-batch":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "same-filename-batch"
        elif argument == "--same-filesize-batch":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "same-filesize-batch"
        elif argument == "--same-fileduration-batch":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "same-fileduration-batch"
        elif argument == "--client-set-file-contract":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "client-set-file-contract"
        elif argument == "--client-user-file-metadata-contract":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "client-user-file-metadata-contract"
        elif argument == "--privacy-file-payload-batch":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "privacy-file-payload-batch"
        elif argument == "--client-chat-send-contract-batch":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "client-chat-send-contract-batch"
        elif argument == "--fanout-batch":
            if mode != "single":
                _emit_json({"Error": {"message": "unknown-argument"}})
                return 2
            mode = "fanout-batch"
        elif argument.startswith("--controlled-room-salt="):
            controlled_room_salt = argument.split("=", 1)[1]
        else:
            _emit_json({"Error": {"message": "unknown-argument"}})
            return 2

    server_version = _load_syncplay_version(legacy_root)
    motd_template = os.environ.get("SYNCPLAY_PROBE_MOTD_TEMPLATE")
    persistent_rooms_enabled = _env_flag_enabled("SYNCPLAY_PROBE_PERSISTENT_ROOMS")
    permanent_rooms = _env_multiline_list("SYNCPLAY_PROBE_PERMANENT_ROOMS")
    tls_available = _env_flag_enabled("SYNCPLAY_PROBE_TLS_AVAILABLE")
    if mode == "fanout-batch":
        return _run_fanout_batch_mode(
            server_version,
            controlled_room_salt,
            motd_template,
            persistent_rooms_enabled,
            permanent_rooms,
            tls_available,
        )
    if mode == "same-filename-batch":
        return _run_same_filename_batch_mode(legacy_root)
    if mode == "same-filesize-batch":
        return _run_same_filesize_batch_mode(legacy_root)
    if mode == "same-fileduration-batch":
        return _run_same_fileduration_batch_mode(legacy_root)
    if mode == "client-set-file-contract":
        return _run_client_set_file_contract_mode(legacy_root)
    if mode == "client-user-file-metadata-contract":
        return _run_client_user_file_metadata_contract_mode(legacy_root)
    if mode == "privacy-file-payload-batch":
        return _run_privacy_file_payload_batch_mode(legacy_root)
    if mode == "client-chat-send-contract-batch":
        return _run_client_chat_send_contract_batch_mode(legacy_root)

    session = ProbeSession(
        server_version,
        motd_template,
        persistent_rooms_enabled,
        permanent_rooms,
        tls_available,
    )
    if mode == "batch":
        return _run_batch_mode(session)
    return _run_single_message_mode(session)


if __name__ == "__main__":
    raise SystemExit(main())
