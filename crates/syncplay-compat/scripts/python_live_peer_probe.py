#!/usr/bin/env python3

import argparse
import json
import os
import socket
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
    return os.path.realpath(os.path.join(script_dir, "..", "..", "..", "syncplay"))


def _add_legacy_root_to_sys_path(legacy_root):
    if legacy_root and legacy_root not in sys.path:
        sys.path.insert(0, legacy_root)


class _RecordingUI:
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


class _RecordingPlaylist:
    def loadPlaylistFromFile(self, _path):
        return None

    def changeToPlaylistIndex(self, _index, _user=None, resetPosition=False):
        return None

    def changePlaylist(self, _files, _user=None):
        return None


class _RecordingClient:
    def __init__(self, username, room):
        self.ui = _RecordingUI()
        self._username = username
        self._room = room
        self._password = None
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
        self.playlist = _RecordingPlaylist()
        self._protocol = None
        self.serverFeatures = {}
        self._clientSupportsTLS = False
        self._serverSupportsTLS = False

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

    def sendFile(self):
        return None

    def setServerVersion(self, _version, features):
        self.serverFeatures = dict(features or {})

    def sendFeaturesToPlayer(self):
        return None

    def addPlayerReadyCallback(self, *_args, **_kwargs):
        return None

    def controllerIdentificationSuccess(self, *_args, **_kwargs):
        return None

    def controllerIdentificationError(self, *_args, **_kwargs):
        return None

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


class _SocketTransport:
    disconnecting = False

    def __init__(self, sock):
        self._socket = sock

    def write(self, data):
        self._socket.sendall(data)

    def writeSequence(self, sequence):
        self._socket.sendall(b"".join(sequence))

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
    parser.add_argument("--timeout-seconds", type=float, default=3.0)
    args = parser.parse_args()

    reader = None
    transport = None
    try:
        legacy_root = _legacy_root()
        _add_legacy_root_to_sys_path(legacy_root)
        from syncplay.protocols import SyncClientProtocol  # type: ignore

        sock = socket.create_connection(
            (args.host, args.port), timeout=args.timeout_seconds
        )
        sock.settimeout(args.timeout_seconds)
        sock.settimeout(0.25)
        reader = sock.makefile("rb")
        transport = _SocketTransport(sock)
        client = _RecordingClient(args.name, args.room)
        protocol = SyncClientProtocol(client)
        protocol.makeConnection(transport)
        ready_event = threading.Event()
        stop_event = threading.Event()
        error_holder = []
        pump_thread = threading.Thread(
            target=_pump_server_lines,
            args=(reader, protocol, ready_event, error_holder, stop_event),
            daemon=True,
        )
        pump_thread.start()
        _wait_for_protocol_ready(ready_event, error_holder, args.timeout_seconds)
        _emit_json(
            {
                "status": "connected",
                "username": client.getUsername(),
                "room": client.getRoom(),
            }
        )
        sys.stdin.read()
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
        if reader is not None:
            try:
                reader.close()
            except OSError:
                pass


if __name__ == "__main__":
    sys.exit(main())
