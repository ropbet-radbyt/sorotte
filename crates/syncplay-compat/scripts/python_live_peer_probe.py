#!/usr/bin/env python3

import argparse
import hashlib
import json
import os
import socket
import ssl
import sys
import threading
import time


def _emit_json(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def _legacy_root():
    env_root = os.environ.get("SYNCPLAY_LEGACY_ROOT")
    if env_root:
        return env_root
    script_dir = os.path.dirname(os.path.realpath(__file__))
    return os.path.realpath(
        os.path.join(
            script_dir, "..", "..", "..", ".interop-cache", "syncplay-legacy"
        )
    )


def _add_legacy_root_to_sys_path(legacy_root):
    if legacy_root and legacy_root not in sys.path:
        sys.path.insert(0, legacy_root)


class _RecordingUI:
    def __init__(self, chat_messages=None):
        self._chat_messages = chat_messages if chat_messages is not None else []

    def showChatMessage(self, username, userMessage):
        self._chat_messages.append(
            {"sender": str(username), "message": str(userMessage)}
        )

    def __getattr__(self, _name):
        def _noop(*_args, **_kwargs):
            return None

        return _noop


class _RecordingUser:
    def __init__(self, username=None, room=None, file_=None):
        self.username = username
        self.room = room
        self.file = file_
        self.ready = None
        self.features = {}
        self.controller = False

    def setReady(self, ready):
        self.ready = ready

    def isReady(self):
        return self.ready

    def setFeatures(self, features):
        self.features = dict(features or {})

    def setControllerStatus(self, controller):
        self.controller = bool(controller)


class _RecordingUserList:
    def __init__(self, username, room):
        self.currentUser = _RecordingUser(username, room)
        self._users = {}

    def clearList(self):
        self._users = {}

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
        if username == self.currentUser.username:
            self.currentUser.room = room
            self.currentUser.file = file_
            self.currentUser.setReady(isReady)
            self.currentUser.setFeatures(features)
            if isController is not None:
                self.currentUser.setControllerStatus(isController)
            return
        user = _RecordingUser(username, room, file_)
        user.setReady(isReady)
        user.setFeatures(features)
        if isController is not None:
            user.setControllerStatus(isController)
        self._users[username] = user

    def modUser(self, username, room, file_):
        if username == self.currentUser.username:
            self.currentUser.room = room
            if file_ is not None:
                self.currentUser.file = file_
            return
        user = self._users.get(username)
        if user is None:
            self.addUser(username, room, file_)
            return
        user.room = room
        if file_ is not None:
            user.file = file_

    def removeUser(self, username):
        self._users.pop(username, None)

    def showUserList(self, _alt_ui=None):
        return None

    def setReady(self, username, isReady):
        if username == self.currentUser.username:
            self.currentUser.setReady(isReady)
            return
        user = self._users.get(username)
        if user is not None:
            user.setReady(isReady)

    def setFeatures(self, username, features):
        if username == self.currentUser.username:
            self.currentUser.setFeatures(features)
            return
        user = self._users.get(username)
        if user is not None:
            user.setFeatures(features)

    def setControllerStatus(self, username, controller):
        if username == self.currentUser.username:
            self.currentUser.setControllerStatus(controller)
            return
        user = self._users.get(username)
        if user is not None:
            user.setControllerStatus(controller)


class _RecordingPlaylist:
    def __init__(self, client):
        self._client = client
        self._playlist = []
        self._playlist_index = None

    def snapshot(self):
        return list(self._playlist), self._playlist_index

    def loadPlaylistFromFile(self, _path):
        return None

    def changeToPlaylistIndex(self, index, user=None, resetPosition=False):
        if index is None:
            return None
        try:
            index = int(index)
        except (TypeError, ValueError):
            return None
        if index < 0 or index >= len(self._playlist):
            return None
        self._playlist_index = index
        if user is None and self._client._protocol is not None:
            self._client._protocol.setPlaylistIndex(index)
        return None

    def changePlaylist(self, files, user=None, resetIndex=False):
        self._playlist = [str(file_) for file_ in (files or [])]
        if not self._playlist:
            self._playlist_index = None
        elif self._playlist_index is None or self._playlist_index >= len(self._playlist):
            self._playlist_index = 0
        if user is None and self._client._protocol is not None:
            self._client._protocol.setPlaylist(self._playlist)
            if self._playlist:
                target_index = 0 if resetIndex or self._playlist_index is None else self._playlist_index
                self.changeToPlaylistIndex(target_index)
        return None


class _RecordingClient:
    def __init__(self, username, room, password=None):
        self.chat_messages = []
        self.ui = _RecordingUI(self.chat_messages)
        self._username = username
        self._room = room
        self._password = hashlib.md5(password.encode("utf-8")).hexdigest() if password else None
        self._features = {
            "featureList": True,
            "chat": True,
            "readiness": True,
            "managedRooms": True,
            "persistentRooms": True,
            "setOthersReadiness": True,
            "sharedPlaylists": False,
            "uiMode": "GUI",
        }
        self._config = {
            "readyAtStart": False,
            "loadPlaylistFromFile": None,
            "sharedPlaylistEnabled": False,
        }
        self.userlist = _RecordingUserList(username, room)
        self.playlist = _RecordingPlaylist(self)
        self._protocol = None
        self.serverFeatures = {}
        self._clientSupportsTLS = False
        self._serverSupportsTLS = False
        self.protocolFactory = _ProbeProtocolFactory(None)

    def enable_tls(self, tls_options):
        self._clientSupportsTLS = True
        self._serverSupportsTLS = True
        self.protocolFactory = _ProbeProtocolFactory(tls_options)

    def initProtocol(self, protocol):
        self._protocol = protocol

    def destroyProtocol(self):
        self._protocol = None

    def getUsername(self):
        return self._username

    def getPassword(self):
        return self._password

    def getRoom(self):
        return self._room

    def getFeatures(self):
        return self._features

    def setUsername(self, username):
        self._username = username
        self.userlist.currentUser.username = username

    def setRoom(self, room):
        self._room = room
        self.userlist.currentUser.room = room

    def connected(self):
        if self._protocol is not None:
            self._protocol.setReady(False, manuallyInitiated=False)
            self.getUserList()

    def sendFile(self):
        return None

    def getUserList(self):
        if self._protocol is not None and getattr(self._protocol, "logged", False):
            self._protocol.sendList()

    def setServerVersion(self, _version, features):
        self.serverFeatures = dict(features or {})

    def sendFeaturesToPlayer(self):
        return None

    def addPlayerReadyCallback(self, *_args, **_kwargs):
        return None

    def controllerIdentificationSuccess(self, username, roomname):
        if roomname == self.getRoom():
            self.userlist.setControllerStatus(username, True)

    def controllerIdentificationError(self, username, roomname):
        if roomname == self.getRoom():
            self.userlist.setControllerStatus(username, False)

    def controlledRoomCreated(self, roomName, controlPassword):
        self.setRoom(roomName)
        self.userlist.currentUser.setControllerStatus(True)

    def removeUser(self, username):
        self.userlist.removeUser(username)

    def setReady(self, username, isReady, manuallyInitiated=True, setBy=None):
        self.userlist.setReady(username, isReady)

    def setUserFeatures(self, username, features):
        self.userlist.setFeatures(username, features)

    def updateGlobalState(self, position, paused, doSeek, setBy, messageAge):
        return None

    def getLocalState(self):
        return None, None, None, False


def _user_ready_snapshot(client):
    users = {client.getUsername(): client.userlist.currentUser.isReady()}
    for username, user in client.userlist._users.items():
        users[username] = user.isReady()
    return users


def _user_room_snapshot(client):
    users = {client.getUsername(): client.getRoom()}
    for username, user in client.userlist._users.items():
        users[username] = user.room
    return users


def _user_controller_snapshot(client):
    users = {client.getUsername(): client.userlist.currentUser.controller}
    for username, user in client.userlist._users.items():
        users[username] = user.controller
    return users


def _chat_message_snapshot(client):
    return list(client.chat_messages)


def _file_name_snapshot(file_):
    if not isinstance(file_, dict):
        return None
    name = file_.get("name")
    if name is None:
        return None
    return str(name)


def _user_file_name_snapshot(client):
    users = {client.getUsername(): _file_name_snapshot(client.userlist.currentUser.file)}
    for username, user in client.userlist._users.items():
        users[username] = _file_name_snapshot(user.file)
    return users


def _playlist_snapshot(client):
    playlist, _ = client.playlist.snapshot()
    return playlist


def _playlist_index_snapshot(client):
    _, playlist_index = client.playlist.snapshot()
    return playlist_index


def _emit_client_snapshot(status, client, extra_payload=None):
    payload = {
        "status": status,
        "username": client.getUsername(),
        "room": client.getRoom(),
        "localReady": client.userlist.currentUser.isReady(),
        "fileName": _file_name_snapshot(client.userlist.currentUser.file),
        "localController": client.userlist.currentUser.controller,
        "users": _user_ready_snapshot(client),
        "rooms": _user_room_snapshot(client),
        "userFiles": _user_file_name_snapshot(client),
        "controllers": _user_controller_snapshot(client),
        "playlist": _playlist_snapshot(client),
        "playlistIndex": _playlist_index_snapshot(client),
        "chatMessages": _chat_message_snapshot(client),
    }
    if extra_payload:
        payload.update(extra_payload)
    _emit_json(payload)


def _ready_for_username(client, username):
    if username == client.getUsername():
        return client.userlist.currentUser.isReady()
    user = client.userlist._users.get(username)
    if user is None:
        return None
    return user.isReady()


def _room_for_username(client, username):
    if username == client.getUsername():
        return client.getRoom()
    user = client.userlist._users.get(username)
    if user is None:
        return None
    return user.room


def _controller_for_username(client, username):
    if username == client.getUsername():
        return client.userlist.currentUser.controller
    user = client.userlist._users.get(username)
    if user is None:
        return None
    return user.controller


def _file_name_for_username(client, username):
    if username == client.getUsername():
        return _file_name_snapshot(client.userlist.currentUser.file)
    user = client.userlist._users.get(username)
    if user is None:
        return None
    return _file_name_snapshot(user.file)


def _wait_for_ready_value(client, getter, expected_ready, timeout_seconds, error_holder):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if error_holder:
            raise RuntimeError(error_holder[0])
        if getter() == expected_ready:
            return
        time.sleep(0.05)
    if error_holder:
        raise RuntimeError(error_holder[0])
    raise RuntimeError("python live peer timed out waiting for the requested readiness state")


def _wait_for_room_value(client, getter, expected_room, timeout_seconds, error_holder):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if error_holder:
            raise RuntimeError(error_holder[0])
        if getter() == expected_room:
            return
        time.sleep(0.05)
    if error_holder:
        raise RuntimeError(error_holder[0])
    raise RuntimeError("python live peer timed out waiting for the requested room state")


def _wait_for_controller_value(
    client, getter, expected_controller, timeout_seconds, error_holder
):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if error_holder:
            raise RuntimeError(error_holder[0])
        if getter() == expected_controller:
            return
        time.sleep(0.05)
    if error_holder:
        raise RuntimeError(error_holder[0])
    raise RuntimeError("python live peer timed out waiting for the requested controller state")


def _wait_for_chat_message(
    client, username, message, timeout_seconds, error_holder
):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if error_holder:
            raise RuntimeError(error_holder[0])
        if any(
            entry.get("sender") == username and entry.get("message") == message
            for entry in client.chat_messages
        ):
            return
        time.sleep(0.05)
    if error_holder:
        raise RuntimeError(error_holder[0])
    raise RuntimeError("python live peer timed out waiting for the requested chat message")


def _wait_for_user_file_name(
    client, username, expected_file_name, timeout_seconds, error_holder
):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if error_holder:
            raise RuntimeError(error_holder[0])
        if _file_name_for_username(client, username) == expected_file_name:
            return
        time.sleep(0.05)
    if error_holder:
        raise RuntimeError(error_holder[0])
    raise RuntimeError("python live peer timed out waiting for the requested user file")


def _wait_for_playlist(client, playlist, timeout_seconds, error_holder):
    deadline = time.monotonic() + timeout_seconds
    expected_playlist = [str(file_) for file_ in playlist]
    while time.monotonic() < deadline:
        if error_holder:
            raise RuntimeError(error_holder[0])
        if _playlist_snapshot(client) == expected_playlist:
            return
        time.sleep(0.05)
    if error_holder:
        raise RuntimeError(error_holder[0])
    raise RuntimeError("python live peer timed out waiting for the requested playlist state")


def _wait_for_playlist_index(client, index, timeout_seconds, error_holder):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if error_holder:
            raise RuntimeError(error_holder[0])
        if _playlist_index_snapshot(client) == index:
            return
        time.sleep(0.05)
    if error_holder:
        raise RuntimeError(error_holder[0])
    raise RuntimeError("python live peer timed out waiting for the requested playlist index")


def _handle_command(client, protocol, command, error_holder):
    if error_holder:
        raise RuntimeError(error_holder[0])
    command_name = command.get("command")
    if command_name == "snapshot":
        _emit_client_snapshot("snapshot", client)
        return
    if command_name == "set_ready":
        if "ready" not in command:
            raise RuntimeError("python live peer set_ready command requires a ready field")
        protocol.setReady(bool(command["ready"]), manuallyInitiated=True)
        _emit_client_snapshot("ready-command-sent", client, {"ready": bool(command["ready"])})
        return
    if command_name == "set_room":
        room = command.get("room")
        if not isinstance(room, str) or not room.strip():
            raise RuntimeError("python live peer set_room command requires a room string")
        protocol.sendRoomSetting(room)
        _emit_client_snapshot("room-command-sent", client, {"room": room})
        return
    if command_name == "set_file":
        file_ = command.get("file")
        if not isinstance(file_, dict):
            raise RuntimeError("python live peer set_file command requires a file object")
        protocol.sendFileSetting(file_)
        _emit_client_snapshot("file-command-sent", client, {"file": file_})
        return
    if command_name == "request_controlled_room":
        room = command.get("room")
        password = command.get("password")
        if not isinstance(room, str) or not room.strip():
            raise RuntimeError(
                "python live peer request_controlled_room command requires a room string"
            )
        if not isinstance(password, str) or not password.strip():
            raise RuntimeError(
                "python live peer request_controlled_room command requires a password string"
            )
        protocol.requestControlledRoom(room, password)
        _emit_client_snapshot(
            "controlled-room-command-sent",
            client,
            {"room": room, "password": password},
        )
        return
    if command_name == "wait_for_local_ready":
        if "ready" not in command:
            raise RuntimeError(
                "python live peer wait_for_local_ready command requires a ready field"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        expected_ready = bool(command["ready"])
        _wait_for_ready_value(
            client,
            lambda: client.userlist.currentUser.isReady(),
            expected_ready,
            timeout_seconds,
            error_holder,
        )
        _emit_client_snapshot("local-ready", client, {"ready": expected_ready})
        return
    if command_name == "wait_for_user_ready":
        username = command.get("username")
        if not isinstance(username, str) or not username.strip():
            raise RuntimeError(
                "python live peer wait_for_user_ready command requires a username string"
            )
        if "ready" not in command:
            raise RuntimeError(
                "python live peer wait_for_user_ready command requires a ready field"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        expected_ready = bool(command["ready"])
        _wait_for_ready_value(
            client,
            lambda: _ready_for_username(client, username),
            expected_ready,
            timeout_seconds,
            error_holder,
        )
        _emit_client_snapshot(
            "user-ready", client, {"usernameObserved": username, "ready": expected_ready}
        )
        return
    if command_name == "wait_for_user_room":
        username = command.get("username")
        if not isinstance(username, str) or not username.strip():
            raise RuntimeError(
                "python live peer wait_for_user_room command requires a username string"
            )
        room = command.get("room")
        if not isinstance(room, str) or not room.strip():
            raise RuntimeError(
                "python live peer wait_for_user_room command requires a room string"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        _wait_for_room_value(
            client,
            lambda: _room_for_username(client, username),
            room,
            timeout_seconds,
            error_holder,
        )
        _emit_client_snapshot(
            "user-room", client, {"usernameObserved": username, "room": room}
        )
        return
    if command_name == "wait_for_local_controller":
        if "controller" not in command:
            raise RuntimeError(
                "python live peer wait_for_local_controller command requires a controller field"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        expected_controller = bool(command["controller"])
        _wait_for_controller_value(
            client,
            lambda: client.userlist.currentUser.controller,
            expected_controller,
            timeout_seconds,
            error_holder,
        )
        _emit_client_snapshot(
            "local-controller", client, {"controller": expected_controller}
        )
        return
    if command_name == "wait_for_user_controller":
        username = command.get("username")
        if not isinstance(username, str) or not username.strip():
            raise RuntimeError(
                "python live peer wait_for_user_controller command requires a username string"
            )
        if "controller" not in command:
            raise RuntimeError(
                "python live peer wait_for_user_controller command requires a controller field"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        expected_controller = bool(command["controller"])
        _wait_for_controller_value(
            client,
            lambda: _controller_for_username(client, username),
            expected_controller,
            timeout_seconds,
            error_holder,
        )
        _emit_client_snapshot(
            "user-controller",
            client,
            {"usernameObserved": username, "controller": expected_controller},
        )
        return
    if command_name == "send_chat_message":
        message = command.get("message")
        if not isinstance(message, str):
            raise RuntimeError(
                "python live peer send_chat_message command requires a message string"
            )
        protocol.sendChatMessage(message)
        _emit_client_snapshot("chat-command-sent", client, {"message": message})
        return
    if command_name == "wait_for_chat_message":
        username = command.get("username")
        if not isinstance(username, str) or not username.strip():
            raise RuntimeError(
                "python live peer wait_for_chat_message command requires a username string"
            )
        message = command.get("message")
        if not isinstance(message, str):
            raise RuntimeError(
                "python live peer wait_for_chat_message command requires a message string"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        _wait_for_chat_message(
            client,
            username,
            message,
            timeout_seconds,
            error_holder,
        )
        _emit_client_snapshot(
            "chat-message",
            client,
            {"usernameObserved": username, "message": message},
        )
        return
    if command_name == "wait_for_user_file_name":
        username = command.get("username")
        if not isinstance(username, str) or not username.strip():
            raise RuntimeError(
                "python live peer wait_for_user_file_name command requires a username string"
            )
        file_name = command.get("fileName")
        if not isinstance(file_name, str) or not file_name.strip():
            raise RuntimeError(
                "python live peer wait_for_user_file_name command requires a fileName string"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        _wait_for_user_file_name(
            client,
            username,
            file_name,
            timeout_seconds,
            error_holder,
        )
        _emit_client_snapshot(
            "user-file",
            client,
            {"usernameObserved": username, "fileName": file_name},
        )
        return
    if command_name == "set_playlist":
        files = command.get("files")
        if not isinstance(files, list) or any(not isinstance(file_, str) for file_ in files):
            raise RuntimeError(
                "python live peer set_playlist command requires a files string list"
            )
        client.playlist.changePlaylist(files, user=None, resetIndex=True)
        _emit_client_snapshot("playlist-command-sent", client, {"files": files})
        return
    if command_name == "set_playlist_index":
        index = command.get("index")
        if not isinstance(index, int):
            raise RuntimeError(
                "python live peer set_playlist_index command requires an integer index"
            )
        client.playlist.changeToPlaylistIndex(index, user=None, resetPosition=False)
        _emit_client_snapshot("playlist-index-command-sent", client, {"index": index})
        return
    if command_name == "wait_for_playlist":
        files = command.get("files")
        if not isinstance(files, list) or any(not isinstance(file_, str) for file_ in files):
            raise RuntimeError(
                "python live peer wait_for_playlist command requires a files string list"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        _wait_for_playlist(client, files, timeout_seconds, error_holder)
        _emit_client_snapshot("playlist", client, {"filesObserved": files})
        return
    if command_name == "wait_for_playlist_index":
        index = command.get("index")
        if not isinstance(index, int):
            raise RuntimeError(
                "python live peer wait_for_playlist_index command requires an integer index"
            )
        timeout_seconds = float(command.get("timeoutSeconds", 3.0))
        _wait_for_playlist_index(client, index, timeout_seconds, error_holder)
        _emit_client_snapshot("playlist-index", client, {"indexObserved": index})
        return
    raise RuntimeError(f"python live peer received an unknown command: {command_name!r}")


def _pump_server_lines(reader, protocol, ready_event, error_holder, stop_event):
    while not stop_event.is_set():
        try:
            raw_line = reader.readline()
        except OSError as exc:
            if not stop_event.is_set():
                error_holder.append(str(exc))
            return
        if not raw_line:
            if not stop_event.is_set():
                error_holder.append("legacy server closed the live peer connection")
            return
        try:
            protocol.lineReceived(raw_line)
        except Exception as exc:  # pragma: no cover - probe-only diagnostic path
            error_holder.append(str(exc))
            return
        if getattr(protocol, "logged", False):
            ready_event.set()


def _wait_for_protocol_ready(ready_event, error_holder, timeout_seconds):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if ready_event.wait(timeout=0.05):
            return
        if error_holder:
            raise RuntimeError(error_holder[0])
    if error_holder:
        raise RuntimeError(error_holder[0])
    raise RuntimeError("python live peer timed out waiting for protocol login")


class _ProbeProtocolFactory:
    def __init__(self, options):
        self.options = options

    def stopRetrying(self):
        return None


class _ProbeTlsOptions:
    def __init__(self, ca_file, hostname):
        self.ca_file = ca_file
        self.hostname = hostname


class _OpenSslConnectionInfo:
    def __init__(self, tls_socket):
        self._tls_socket = tls_socket

    def get_protocol_version(self):
        version = self._tls_socket.version()
        if version == "TLSv1.2":
            return 771
        if version == "TLSv1.3":
            return 772
        return version

    def get_protocol_version_name(self):
        return self._tls_socket.version() or "unknown"

    def get_cipher_name(self):
        cipher = self._tls_socket.cipher()
        return cipher[0] if cipher else "unknown"


class _ProbeTransportProtocol:
    def __init__(self):
        self._tlsConnection = None


class _SocketTransport:
    disconnecting = False

    def __init__(self, sock):
        self._socket = sock
        self._syncplay_protocol = None
        self._peer_certificate = None
        self.protocol = _ProbeTransportProtocol()

    def set_syncplay_protocol(self, protocol):
        self._syncplay_protocol = protocol

    def readline(self):
        chunks = []
        while True:
            chunk = self._socket.recv(1)
            if not chunk:
                return b"".join(chunks)
            chunks.append(chunk)
            if chunk == b"\n":
                return b"".join(chunks)

    def write(self, data):
        self._socket.sendall(data)

    def writeSequence(self, sequence):
        self._socket.sendall(b"".join(sequence))

    def startTLS(self, options):
        if options is None:
            raise RuntimeError("python live peer TLS requested without TLS options")
        if not options.ca_file:
            raise RuntimeError("python live peer TLS requested without a CA file")
        context = ssl.create_default_context(cafile=options.ca_file)
        context.check_hostname = True
        tls_socket = context.wrap_socket(
            self._socket,
            server_hostname=options.hostname,
        )
        self._socket = tls_socket
        self.protocol._tlsConnection = _OpenSslConnectionInfo(tls_socket)

        from OpenSSL import crypto  # type: ignore

        peer_certificate = tls_socket.getpeercert(binary_form=True)
        if not peer_certificate:
            raise RuntimeError("python live peer TLS server certificate was not available")
        self._peer_certificate = crypto.load_certificate(
            crypto.FILETYPE_ASN1, peer_certificate
        )
        if self._syncplay_protocol is None:
            raise RuntimeError("python live peer TLS transport has no protocol")
        self._syncplay_protocol.handshakeCompleted()

    def getPeerCertificate(self):
        return self._peer_certificate

    def loseConnection(self):
        self.disconnecting = True
        try:
            self._socket.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self._socket.close()

    abortConnection = loseConnection


def _wait_for_first_server_line(reader, timeout_seconds):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        raw_line = reader.readline()
        if not raw_line:
            raise RuntimeError(
                "python live peer did not receive a server response after sending hello"
            )
        line = raw_line.decode("utf-8", "replace").strip()
        if line:
            return line
    raise RuntimeError("python live peer timed out waiting for a server response line")


def main():
    parser = argparse.ArgumentParser(description="Connect a lightweight Python Syncplay peer.")
    parser.add_argument("--host", required=True)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--name", required=True)
    parser.add_argument("--room", required=True)
    parser.add_argument("--password")
    parser.add_argument("--tls", action="store_true")
    parser.add_argument("--tls-ca-file")
    parser.add_argument("--tls-hostname", default="localhost")
    parser.add_argument("--timeout-seconds", type=float, default=3.0)
    args = parser.parse_args()

    transport = None
    try:
        legacy_root = _legacy_root()
        _add_legacy_root_to_sys_path(legacy_root)
        from syncplay.protocols import SyncClientProtocol  # type: ignore

        sock = socket.create_connection(
            (args.host, args.port), timeout=args.timeout_seconds
        )
        # Use connect-time timeout only; once attached, keep the read loop blocking
        # so idle periods do not surface as fatal probe errors between commands.
        sock.settimeout(None)
        transport = _SocketTransport(sock)
        client = _RecordingClient(args.name, args.room, password=args.password)
        if args.tls:
            if not args.tls_ca_file:
                raise RuntimeError("python live peer --tls requires --tls-ca-file")
            client.enable_tls(_ProbeTlsOptions(args.tls_ca_file, args.tls_hostname))
        protocol = SyncClientProtocol(client)
        transport.set_syncplay_protocol(protocol)
        protocol.makeConnection(transport)
        ready_event = threading.Event()
        stop_event = threading.Event()
        error_holder = []
        pump_thread = threading.Thread(
            target=_pump_server_lines,
            args=(transport, protocol, ready_event, error_holder, stop_event),
            daemon=True,
        )
        pump_thread.start()
        _wait_for_protocol_ready(ready_event, error_holder, args.timeout_seconds)
        _emit_client_snapshot("connected", client)
        for raw_command in sys.stdin:
            raw_command = raw_command.strip()
            if not raw_command:
                continue
            command = json.loads(raw_command)
            _handle_command(client, protocol, command, error_holder)
        stop_event.set()
        transport.loseConnection()
        pump_thread.join(timeout=1.0)
        return 0
    except Exception as exc:
        _emit_json({"status": "error", "error": str(exc)})
        if transport is not None:
            transport.loseConnection()
        return 1
    finally:
        pass


if __name__ == "__main__":
    sys.exit(main())
