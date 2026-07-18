local utils = require "mp.utils"

local SCRIPT_NAME = "sorotte_network_options"
local PROTOCOL = "sorotte-network-options-v1"
local CONFIGURE_MESSAGE = "sorotte_network_options_configure"
local APPLY_ACTIVE_MESSAGE = "sorotte_network_options_apply_active"
local CONFIGURED_MESSAGE = "sorotte-network-options-configured"
local ACTIVE_RESULT_MESSAGE = "sorotte-network-options-active-result"
local TRANSITION_RESULT_MESSAGE = "sorotte-network-options-transition-result"

-- `load-script` gives duplicate clients a suffixed name. Only the stable canonical client owns
-- core policy so reconnecting Sorotte processes cannot install duplicate on-load hooks.
local script_name = mp.get_script_name ~= nil and mp.get_script_name() or mp.script_name
if script_name ~= SCRIPT_NAME then
    mp.keep_running = false
    return
end

local generation = 0
local attachment = nil
local options = {}
local last_active_attempt = nil
local last_active_result = nil

local function emit(name, payload)
    mp.commandv("script-message", name, utils.format_json(payload))
end

local function network_path(path)
    if type(path) ~= "string" then return false end
    local scheme = path:match("^%s*([^:]+)://")
    return scheme ~= nil and scheme:lower() ~= "file"
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

local function result_payload(status, path, error_message)
    local payload = {
        protocol = PROTOCOL,
        attachment = attachment,
        generation = generation,
        status = status,
        path = path,
    }
    if error_message ~= nil then payload.error = tostring(error_message) end
    return payload
end

mp.register_script_message(CONFIGURE_MESSAGE, function(payload_text)
    local payload = utils.parse_json(payload_text)
    if type(payload) ~= "table" or payload.protocol ~= PROTOCOL then return end
    if type(payload.attachment) ~= "string" or payload.attachment == "" then return end
    if type(payload.generation) ~= "number" or type(payload.options) ~= "table" then return end

    if payload.attachment ~= attachment then
        attachment = payload.attachment
        generation = 0
        last_active_attempt = nil
        last_active_result = nil
    end
    if payload.attachment == attachment and payload.generation >= generation then
        generation = payload.generation
        options = payload.options
        last_active_attempt = nil
        last_active_result = nil
    end
    emit(CONFIGURED_MESSAGE, {
        protocol = PROTOCOL,
        attachment = payload.attachment,
        generation = payload.generation,
        status = payload.generation == generation and "configured" or "stale",
    })
end)

mp.register_script_message(APPLY_ACTIVE_MESSAGE, function(payload_text)
    local payload = utils.parse_json(payload_text)
    if type(payload) ~= "table" or payload.protocol ~= PROTOCOL then return end
    if payload.attachment ~= attachment then return end
    if type(payload.generation) ~= "number" or type(payload.attempt) ~= "number" then return end

    if last_active_attempt == payload.attempt and last_active_result ~= nil then
        emit(ACTIVE_RESULT_MESSAGE, last_active_result)
        return
    end

    local path = mp.get_property("path", "")
    local status, error_message
    if payload.generation ~= generation then
        status = "failed"
        error_message = "network-options configuration generation changed"
    else
        status, error_message = apply_options(path)
    end
    local result = result_payload(status, path, error_message)
    result.attempt = payload.attempt
    last_active_attempt = payload.attempt
    last_active_result = result
    emit(ACTIVE_RESULT_MESSAGE, result)
end)

-- This hook runs synchronously before mpv opens the file. The path classification and all
-- file-local writes therefore happen in one mpv event-loop callback and cannot cross into a
-- superseding local file between JSON IPC commands.
mp.add_hook("on_load", 50, function()
    if generation == 0 then return end
    local path = mp.get_property("stream-open-filename", "")
    if not network_path(path) then return end
    local status, error_message = apply_options(path)
    emit(TRANSITION_RESULT_MESSAGE, result_payload(status, path, error_message))
end)
