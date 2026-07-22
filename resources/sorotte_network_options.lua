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
local OPTION_APPLICATION_ORDER = {
    "cache",
    "cache-pause",
    "cache-pause-initial",
    "cache-pause-wait",
    "cache-secs",
    "demuxer-max-bytes",
    "demuxer-max-back-bytes",
    "cache-on-disk",
    "ytdl-format",
}
local EFFECTIVE_READBACK_ORDER = {
    "cache",
    "cache-pause",
    "cache-pause-initial",
    "cache-pause-wait",
    "cache-secs",
    "demuxer-max-bytes",
    "demuxer-max-back-bytes",
    "cache-on-disk",
}
local CURL_HTTP_VERSION_OPTION = "options/curl-http-version"
local CURL_HTTP_VERSION_FILE_OPTION = "file-local-options/curl-http-version"
local CURL_HTTP_VERSION_AUTOMATIC = "auto"
local CURL_HTTP_VERSION_HTTP2_TLS = "2tls"

-- `load-script` gives duplicate clients a suffixed name. Only the stable canonical client owns
-- core policy so reconnecting Sorotte processes cannot install duplicate on-load hooks.
local script_name = mp.get_script_name ~= nil and mp.get_script_name() or mp.script_name
if script_name ~= SCRIPT_NAME then
    mp.keep_running = false
    return
end

-- Stable for this canonical Lua client lifetime. Rust pairs it with the attachment id so a
-- delivery retry preserves the sequence floor while a genuinely reloaded hook starts a new one.
local hook_instance_anchor = {}
local hook_instance_pid = utils.getpid ~= nil and utils.getpid() or "unknown"
local hook_instance_id = table.concat({
    SCRIPT_NAME,
    tostring(hook_instance_pid),
    tostring(os.time()),
    string.format("%.17g", mp.get_time()),
    tostring(hook_instance_anchor),
}, ":")

local owner_id = nil
local attachment_id = nil
local generation = 0
local options = {}
local owner_last_seen = nil
local owner_lease_seconds = 0
local last_active_attempt = nil
local last_active_result = nil
local load_sequence = 0
local pending_load_verification = nil

local option_application_rank = {}
for index, name in ipairs(OPTION_APPLICATION_ORDER) do
    option_application_rank[name] = index
end

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
    pending_load_verification = nil
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

local function ordered_option_names()
    local names = {}
    for name, _ in pairs(options) do
        table.insert(names, name)
    end
    table.sort(names, function(left, right)
        local left_rank = option_application_rank[left]
        local right_rank = option_application_rank[right]
        if left_rank ~= nil or right_rank ~= nil then
            if left_rank == nil then return false end
            if right_rank == nil then return true end
            return left_rank < right_rank
        end
        return left < right
    end)
    return names
end

-- Newer mpv builds can route HTTP through libcurl. Its automatic protocol selection may choose
-- HTTP/3 for YouTube range requests, but a draining QUIC connection can be surfaced as a clean
-- EOF after curl's bounded retries. Prefer negotiated HTTP/2 for network media when that option
-- exists and the user has not selected a protocol explicitly. Stable mpv builds without the
-- curl backend expose no such option, so this remains a no-op there.
local function apply_curl_transport_safety()
    local current = mp.get_property(CURL_HTTP_VERSION_OPTION, nil)
    if current ~= CURL_HTTP_VERSION_AUTOMATIC then return end
    mp.set_property(CURL_HTTP_VERSION_FILE_OPTION, CURL_HTTP_VERSION_HTTP2_TLS)
end

local function apply_options(path)
    if path == nil or path == "" then return "no-active", {} end
    if not network_path(path) then return "local", {} end

    apply_curl_transport_safety()
    local results = {}
    local applied = 0
    local rejected = 0
    for _, name in ipairs(ordered_option_names()) do
        local ok = mp.set_property("file-local-options/" .. name, options[name])
        local status = "rejected"
        if ok then
            status = "applied"
            applied = applied + 1
        else
            rejected = rejected + 1
        end
        table.insert(results, { name = name, status = status })
    end
    if rejected == 0 then return "network-updated", results end
    if applied == 0 then return "failed", results end
    return "partially-applied", results
end

local function effective_options()
    local effective = {}
    for _, name in ipairs(EFFECTIVE_READBACK_ORDER) do
        if options[name] ~= nil then
            local value = mp.get_property(name, nil)
            if value ~= nil then effective[name] = tostring(value) end
        end
    end
    return effective
end

local function result_payload(status, sequence, source_path, stream_open_filename, option_results)
    local wire_status = status
    local application_state = nil
    if status == "network-updated" then
        application_state = "applied"
    elseif status == "partially-applied" then
        -- Protocol v3 adapters only know `failed` and must continue to fail closed. The additive
        -- field lets newer adapters retain the more precise aggregate without introducing an
        -- incompatible v3 status value.
        wire_status = "failed"
        application_state = "partially-applied"
    elseif status == "failed" then
        application_state = "failed"
    end
    local payload = {
        protocol = PROTOCOL,
        ownerId = owner_id,
        attachmentId = attachment_id,
        configurationGeneration = generation,
        hookInstanceId = hook_instance_id,
        loadSequence = sequence,
        sourcePath = source_path,
        streamOpenFilename = stream_open_filename,
        status = wire_status,
        optionResults = option_results,
    }
    if application_state ~= nil then payload.applicationState = application_state end
    return payload
end

local function verified_result_payload(
    status,
    sequence,
    source_path,
    stream_open_filename,
    option_results
)
    local payload = result_payload(
        status,
        sequence,
        source_path,
        stream_open_filename,
        option_results
    )
    payload.verification = "complete"
    payload.effectiveOptions = effective_options()
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
        pending_load_verification = nil
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
    local status, option_results
    if payload.configurationGeneration ~= generation then
        status = "failed"
        option_results = {}
    else
        status, option_results = apply_options(stream_open_filename)
    end
    local result = verified_result_payload(
        status,
        load_sequence,
        source_path,
        stream_open_filename,
        option_results
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
    local status, option_results = apply_options(stream_open_filename)
    if status == "no-active" or status == "local" then
        pending_load_verification = nil
        emit(TRANSITION_RESULT_MESSAGE, result_payload(
            status,
            load_sequence,
            source_path,
            stream_open_filename,
            option_results
        ))
        return
    end
    pending_load_verification = {
        status = status,
        sequence = load_sequence,
        sourcePath = source_path,
        streamOpenFilename = stream_open_filename,
        optionResults = option_results,
    }
end)

mp.register_event("file-loaded", function()
    expire_owner_if_needed()
    if not owner_is_live() or pending_load_verification == nil then return end
    local pending = pending_load_verification
    pending_load_verification = nil
    emit(TRANSITION_RESULT_MESSAGE, verified_result_payload(
        pending.status,
        pending.sequence,
        pending.sourcePath,
        pending.streamOpenFilename,
        pending.optionResults
    ))
end)

mp.register_event("end-file", function()
    expire_owner_if_needed()
    if not owner_is_live() or pending_load_verification == nil then return end
    local pending = pending_load_verification
    pending_load_verification = nil
    local payload = result_payload(
        "failed",
        pending.sequence,
        pending.sourcePath,
        pending.streamOpenFilename,
        pending.optionResults
    )
    payload.verification = "incomplete"
    emit(TRANSITION_RESULT_MESSAGE, payload)
end)

mp.add_periodic_timer(0.1, expire_owner_if_needed)
