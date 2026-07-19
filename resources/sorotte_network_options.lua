local utils = require "mp.utils"

local SCRIPT_NAME = "sorotte_network_options"
local PROTOCOL = "sorotte-network-options-v3"
local CONFIGURE_MESSAGE = "sorotte_network_options_configure"
local HEARTBEAT_MESSAGE = "sorotte_network_options_heartbeat"
local RELEASE_MESSAGE = "sorotte_network_options_release"
local APPLY_ACTIVE_MESSAGE = "sorotte_network_options_apply_active"
local CONFIGURED_MESSAGE = "sorotte-network-options-configured"
local OWNERSHIP_MESSAGE = "sorotte-network-options-ownership"
local HEARTBEAT_RESULT_MESSAGE = "sorotte-network-options-heartbeat"
local ACTIVE_RESULT_MESSAGE = "sorotte-network-options-active-result"
local TRANSITION_RESULT_MESSAGE = "sorotte-network-options-transition-result"
local MINIMUM_LEASE_MS = 250
local MAXIMUM_LEASE_MS = 30000

-- `load-script` gives duplicate clients a suffixed name. Only the stable canonical client owns
-- core policy so reconnecting Sorotte processes cannot install duplicate on-load hooks.
local script_name = mp.get_script_name ~= nil and mp.get_script_name() or mp.script_name
if script_name ~= SCRIPT_NAME then
    mp.keep_running = false
    return
end

-- Stable for this canonical Lua client lifetime. Rust pairs it with the attachment id so a
-- delivery retry preserves the sequence floor while a genuinely reloaded hook starts a new one.
local hook_instance_id = SCRIPT_NAME .. ":" .. tostring({})

local owner_id = nil
local attachment_id = nil
local generation = 0
local options = {}
local owner_last_seen = nil
local owner_lease_seconds = 0
local last_active_attempt = nil
local last_active_result = nil
local load_sequence = 0

local function emit(name, payload)
    mp.commandv("script-message", name, utils.format_json(payload))
end

local function network_path(path)
    if type(path) ~= "string" then return false end
    local scheme = path:match("^%s*([^:]+)://")
    return scheme ~= nil and scheme:lower() ~= "file"
end

local function clear_owner()
    owner_id = nil
    attachment_id = nil
    generation = 0
    options = {}
    owner_last_seen = nil
    owner_lease_seconds = 0
    last_active_attempt = nil
    last_active_result = nil
end

local function owner_is_live()
    return owner_id ~= nil
        and attachment_id ~= nil
        and owner_last_seen ~= nil
        and mp.get_time() - owner_last_seen < owner_lease_seconds
end

local function ownership_payload(status, target_owner_id, target_attachment_id, target_generation)
    return {
        protocol = PROTOCOL,
        ownerId = target_owner_id,
        attachmentId = target_attachment_id,
        configurationGeneration = target_generation,
        hookInstanceId = hook_instance_id,
        currentLoadSequence = load_sequence,
        status = status,
    }
end

local function expire_owner_if_needed()
    if owner_id == nil or owner_is_live() then return false end
    local expired_owner_id = owner_id
    local expired_attachment_id = attachment_id
    local expired_generation = generation
    clear_owner()
    emit(OWNERSHIP_MESSAGE, ownership_payload(
        "lease-expired",
        expired_owner_id,
        expired_attachment_id,
        expired_generation
    ))
    return true
end

local function apply_options(path)
    if path == nil or path == "" then return "no-active", nil end
    if not network_path(path) then return "local", nil end

    for name, value in pairs(options) do
        local ok, error_message = mp.set_property("file-local-options/" .. name, value)
        if not ok then
            return "failed", error_message or ("mpv rejected file-local option " .. name)
        end
    end
    return "network-updated", nil
end

local function result_payload(status, sequence, source_path, stream_open_filename, error_message)
    local payload = {
        protocol = PROTOCOL,
        ownerId = owner_id,
        attachmentId = attachment_id,
        configurationGeneration = generation,
        hookInstanceId = hook_instance_id,
        loadSequence = sequence,
        sourcePath = source_path,
        streamOpenFilename = stream_open_filename,
        status = status,
    }
    if error_message ~= nil then payload.error = tostring(error_message) end
    return payload
end

local function valid_controller_payload(payload)
    return type(payload) == "table"
        and payload.protocol == PROTOCOL
        and type(payload.ownerId) == "string"
        and payload.ownerId ~= ""
        and type(payload.attachmentId) == "string"
        and payload.attachmentId ~= ""
end

mp.register_script_message(CONFIGURE_MESSAGE, function(payload_text)
    local payload = utils.parse_json(payload_text)
    if not valid_controller_payload(payload)
        or type(payload.configurationGeneration) ~= "number"
        or type(payload.leaseMs) ~= "number"
        or type(payload.options) ~= "table"
    then
        return
    end

    expire_owner_if_needed()
    if owner_is_live()
        and (payload.ownerId ~= owner_id or payload.attachmentId ~= attachment_id)
    then
        local rejected = ownership_payload(
            "owner-live",
            payload.ownerId,
            payload.attachmentId,
            payload.configurationGeneration
        )
        rejected.activeOwnerId = owner_id
        rejected.activeAttachmentId = attachment_id
        emit(CONFIGURED_MESSAGE, rejected)
        return
    end

    local same_attachment = payload.ownerId == owner_id
        and payload.attachmentId == attachment_id
    local status = "configured"
    if same_attachment and payload.configurationGeneration < generation then
        status = "stale"
    else
        owner_id = payload.ownerId
        attachment_id = payload.attachmentId
        generation = payload.configurationGeneration
        options = payload.options
        last_active_attempt = nil
        last_active_result = nil
    end
    owner_lease_seconds = math.max(
        MINIMUM_LEASE_MS,
        math.min(MAXIMUM_LEASE_MS, payload.leaseMs)
    ) / 1000
    owner_last_seen = mp.get_time()
    emit(CONFIGURED_MESSAGE, ownership_payload(
        status,
        payload.ownerId,
        payload.attachmentId,
        payload.configurationGeneration
    ))
end)

mp.register_script_message(HEARTBEAT_MESSAGE, function(payload_text)
    local payload = utils.parse_json(payload_text)
    if not valid_controller_payload(payload)
        or type(payload.configurationGeneration) ~= "number"
        or type(payload.heartbeatNonce) ~= "number"
    then
        return
    end
    expire_owner_if_needed()
    if payload.ownerId ~= owner_id
        or payload.attachmentId ~= attachment_id
        or payload.configurationGeneration ~= generation
    then
        emit(OWNERSHIP_MESSAGE, ownership_payload(
            "ownership-lost",
            payload.ownerId,
            payload.attachmentId,
            payload.configurationGeneration
        ))
        return
    end
    owner_last_seen = mp.get_time()
    local acknowledged = ownership_payload(
        "renewed",
        payload.ownerId,
        payload.attachmentId,
        generation
    )
    acknowledged.heartbeatNonce = payload.heartbeatNonce
    emit(HEARTBEAT_RESULT_MESSAGE, acknowledged)
end)

mp.register_script_message(RELEASE_MESSAGE, function(payload_text)
    local payload = utils.parse_json(payload_text)
    if not valid_controller_payload(payload) then return end
    expire_owner_if_needed()
    if payload.ownerId ~= owner_id or payload.attachmentId ~= attachment_id then return end
    local released_generation = generation
    clear_owner()
    emit(OWNERSHIP_MESSAGE, ownership_payload(
        "released",
        payload.ownerId,
        payload.attachmentId,
        released_generation
    ))
end)

mp.register_script_message(APPLY_ACTIVE_MESSAGE, function(payload_text)
    local payload = utils.parse_json(payload_text)
    if not valid_controller_payload(payload)
        or type(payload.configurationGeneration) ~= "number"
        or type(payload.attempt) ~= "number"
    then
        return
    end

    expire_owner_if_needed()
    if payload.ownerId ~= owner_id or payload.attachmentId ~= attachment_id then
        local lost = ownership_payload(
            "ownership-lost",
            payload.ownerId,
            payload.attachmentId,
            payload.configurationGeneration
        )
        lost.attempt = payload.attempt
        lost.sourcePath = mp.get_property("path", "")
        lost.streamOpenFilename = mp.get_property("stream-open-filename", lost.sourcePath)
        lost.loadSequence = load_sequence
        lost.error = "network-options hook ownership was lost"
        lost.status = "failed"
        emit(ACTIVE_RESULT_MESSAGE, lost)
        return
    end

    if last_active_attempt == payload.attempt and last_active_result ~= nil then
        emit(ACTIVE_RESULT_MESSAGE, last_active_result)
        return
    end

    local source_path = mp.get_property("path", "")
    local stream_open_filename = mp.get_property("stream-open-filename", source_path)
    if stream_open_filename == "" then stream_open_filename = source_path end
    local status, error_message
    if payload.configurationGeneration ~= generation then
        status = "failed"
        error_message = "network-options configuration generation changed"
    else
        status, error_message = apply_options(stream_open_filename)
    end
    local result = result_payload(
        status,
        load_sequence,
        source_path,
        stream_open_filename,
        error_message
    )
    result.attempt = payload.attempt
    last_active_attempt = payload.attempt
    last_active_result = result
    emit(ACTIVE_RESULT_MESSAGE, result)
end)

-- This hook runs synchronously before mpv opens the file. The path classification and all
-- file-local writes therefore happen in one mpv event-loop callback and cannot cross into a
-- superseding local file between JSON IPC commands. Local media emits an explicit completion so
-- an older superseded network attempt cannot remain authoritative.
mp.add_hook("on_load", 50, function()
    expire_owner_if_needed()
    if not owner_is_live() then return end
    load_sequence = load_sequence + 1
    local source_path = mp.get_property("path", "")
    local stream_open_filename = mp.get_property("stream-open-filename", source_path)
    if stream_open_filename == "" then stream_open_filename = source_path end
    local status, error_message = apply_options(stream_open_filename)
    emit(TRANSITION_RESULT_MESSAGE, result_payload(
        status,
        load_sequence,
        source_path,
        stream_open_filename,
        error_message
    ))
end)

mp.add_periodic_timer(0.1, expire_owner_if_needed)
