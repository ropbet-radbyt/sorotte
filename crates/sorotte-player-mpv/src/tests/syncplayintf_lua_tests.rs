use mlua::{Function, Lua, LuaSerdeExt, Table, Value};
use serde_json::{Value as JsonValue, json};

const SOROTTE_PROTOCOL: &str = "sorotte-syncplayintf-v1";
const OPTIONS_MESSAGE: &str = "set_sorotte_syncplayintf_options";
const OPTIONS_ACK_MESSAGE: &str = "sorotte-syncplayintf-options-applied";
const PING_MESSAGE: &str = "sorotte_syncplayintf_ping";
const PONG_MESSAGE: &str = "sorotte-syncplayintf-pong";
const HEARTBEAT_MESSAGE: &str = "sorotte_syncplayintf_heartbeat";
const RELEASE_MESSAGE: &str = "sorotte_syncplayintf_release";
const LEASE_EXPIRED_MESSAGE: &str = "sorotte-syncplayintf-lease-expired";
const CHAT_MESSAGE: &str = "syncplayintf-chat";
const CANONICAL_SCRIPT_NAME: &str = "sorotte_syncplayintf";

const SCRIPT_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/sorotte_syncplayintf.lua"
));

const MP_MOCK_SOURCE: &str = r#"
__harness = {
    time = 0,
    messages = {},
    bindings = {},
    captured_bindings = {},
    binding_add_counts = {},
    binding_remove_counts = {},
    sections = {},
    section_define_counts = {},
    emissions = {},
    commands = {},
    commandvs = {},
    properties = {},
    osd = '',
}

mp = { script_name = __script_name, keep_running = true }

function mp.get_script_name()
    return mp.script_name
end

function mp.get_time()
    return __harness.time
end

function mp.register_script_message(name, callback)
    __harness.messages[name] = callback
end

function mp.add_periodic_timer(interval, callback)
    __harness.periodic_interval = interval
    __harness.periodic_callback = callback
    return { kill = function() end }
end

function mp.observe_property(name, kind, callback)
    __harness.observers = __harness.observers or {}
    __harness.observers[name] = { kind = kind, callback = callback }
end

function mp.get_property_native(name, default)
    local value = __harness.properties[name]
    if value == nil then return default end
    return value
end

function mp.set_osd_ass(width, height, text)
    __harness.osd_width = width
    __harness.osd_height = height
    __harness.osd = text
end

function mp.command(command)
    table.insert(__harness.commands, command)
end

function mp.commandv(...)
    local args = {...}
    table.insert(__harness.commandvs, args)
    if args[1] == 'define-section' then
        local name = args[2]
        local section = __harness.sections[name] or {}
        section.config = args[3]
        section.flags = args[4]
        section.enabled = section.enabled == true
        __harness.sections[name] = section
        __harness.section_define_counts[name] =
            (__harness.section_define_counts[name] or 0) + 1
    elseif args[1] == 'script-message' then
        table.insert(__harness.emissions, {
            name = args[2],
            payload = args[3],
            input_enabled = opts ~= nil and opts['chatInputEnabled'] == true,
            direct_input = opts ~= nil and opts['chatDirectInput'] == true,
            enter_present = __harness.bindings['sorotte-chat-enter'] ~= nil,
            kp_enter_present = __harness.bindings['sorotte-chat-kp-enter'] ~= nil,
            tab_present = __harness.bindings['sorotte-chat-direct-tab'] ~= nil,
            alpha_enabled = __harness.sections['repl-alpha-input'] ~= nil
                and __harness.sections['repl-alpha-input'].enabled == true,
        })
    end
end

function mp.add_forced_key_binding(key, name, callback, flags)
    __harness.bindings[name] = {
        key = key,
        callback = callback,
        flags = flags,
    }
    __harness.binding_add_counts[name] =
        (__harness.binding_add_counts[name] or 0) + 1
end

function mp.remove_key_binding(name)
    __harness.bindings[name] = nil
    __harness.binding_remove_counts[name] =
        (__harness.binding_remove_counts[name] or 0) + 1
end

function mp.enable_key_bindings(name, flags)
    local section = __harness.sections[name] or {}
    section.enabled = true
    section.enable_flags = flags
    __harness.sections[name] = section
end

function mp.disable_key_bindings(name)
    local section = __harness.sections[name] or {}
    section.enabled = false
    __harness.sections[name] = section
end

function __harness.capture_binding(name, capture_name)
    local binding = __harness.bindings[name]
    assert(binding ~= nil, 'binding not present: ' .. name)
    __harness.captured_bindings[capture_name] = binding.callback
end

function __harness.invoke_binding(name, ...)
    local binding = __harness.bindings[name]
    assert(binding ~= nil, 'binding not present: ' .. name)
    return binding.callback(...)
end

function __harness.invoke_captured_binding(capture_name, ...)
    local callback = __harness.captured_bindings[capture_name]
    assert(callback ~= nil, 'captured binding not present: ' .. capture_name)
    return callback(...)
end

package.preload['mp.assdraw'] = function()
    return {
        ass_new = function()
            local ass = { text = '' }
            function ass:append(value)
                if type(value) == 'table' then value = value.text end
                self.text = self.text .. tostring(value or '')
            end
            return ass
        end,
    }
end

package.preload['mp.options'] = function()
    return { read_options = function() end }
end

package.preload['mp.utils'] = function()
    return {
        parse_json = __parse_json,
        format_json = __format_json,
        subprocess = function()
            return { error = true, stdout = '' }
        end,
    }
end
"#;

struct LuaHarness {
    lua: Lua,
    bridge_instance_id: String,
}

fn load_script_with_client_name(script_name: &str) -> mlua::Result<Lua> {
    let lua = Lua::new();
    let parse_json = lua.create_function(|lua, input: String| {
        let value: JsonValue = serde_json::from_str(&input).map_err(mlua::Error::external)?;
        lua.to_value(&value)
    })?;
    let format_json = lua.create_function(|lua, input: Value| {
        let value: JsonValue = lua.from_value(input)?;
        serde_json::to_string(&value).map_err(mlua::Error::external)
    })?;
    lua.globals().set("__parse_json", parse_json)?;
    lua.globals().set("__format_json", format_json)?;
    lua.globals().set("__script_name", script_name)?;
    lua.load(MP_MOCK_SOURCE).set_name("mp_mock").exec()?;
    lua.load(SCRIPT_SOURCE)
        .set_name("sorotte_syncplayintf.lua")
        .exec()?;
    Ok(lua)
}

impl LuaHarness {
    fn new() -> mlua::Result<Self> {
        let lua = load_script_with_client_name(CANONICAL_SCRIPT_NAME)?;
        let mut harness = Self {
            lua,
            bridge_instance_id: String::new(),
        };
        harness.trigger_json(
            PING_MESSAGE,
            &json!({"protocol": SOROTTE_PROTOCOL, "nonce": "harness-discovery"}),
        )?;
        harness.bridge_instance_id = harness
            .payloads(PONG_MESSAGE)?
            .last()
            .and_then(|payload| payload.get("bridgeInstanceId"))
            .and_then(JsonValue::as_str)
            .expect("the real script should answer discovery with its bridge identity")
            .to_owned();
        Ok(harness)
    }

    fn harness_table(&self) -> mlua::Result<Table> {
        self.lua.globals().get("__harness")
    }

    fn trigger_json(&self, name: &str, payload: &JsonValue) -> mlua::Result<()> {
        self.trigger_text(
            name,
            &serde_json::to_string(payload).expect("JSON payload should encode"),
        )
    }

    fn trigger_text(&self, name: &str, text: &str) -> mlua::Result<()> {
        let messages: Table = self.harness_table()?.get("messages")?;
        let callback: Function = messages.get(name)?;
        callback.call(text)
    }

    fn apply(
        &self,
        owner_id: &str,
        attachment_id: &str,
        generation: u64,
        settings: &JsonValue,
    ) -> mlua::Result<()> {
        self.trigger_json(
            OPTIONS_MESSAGE,
            &json!({
                "protocol": SOROTTE_PROTOCOL,
                "bridgeInstanceId": self.bridge_instance_id,
                "ownerId": owner_id,
                "attachmentId": attachment_id,
                "generation": generation,
                "leaseMs": 2_000,
                "settings": settings,
            }),
        )
    }

    fn controller_payload(&self, owner_id: &str, attachment_id: &str) -> JsonValue {
        json!({
            "protocol": SOROTTE_PROTOCOL,
            "bridgeInstanceId": self.bridge_instance_id,
            "ownerId": owner_id,
            "attachmentId": attachment_id,
        })
    }

    fn payloads(&self, name: &str) -> mlua::Result<Vec<JsonValue>> {
        let emissions: Table = self.harness_table()?.get("emissions")?;
        let mut payloads = Vec::new();
        for emission in emissions.sequence_values::<Table>() {
            let emission = emission?;
            if emission.get::<String>("name")? == name {
                let payload: String = emission.get("payload")?;
                payloads.push(
                    serde_json::from_str(&payload)
                        .expect("the real script should emit valid JSON payloads"),
                );
            }
        }
        Ok(payloads)
    }

    fn last_emission_snapshot(&self, name: &str) -> mlua::Result<JsonValue> {
        let emissions: Table = self.harness_table()?.get("emissions")?;
        let mut result = None;
        for emission in emissions.sequence_values::<Table>() {
            let emission = emission?;
            if emission.get::<String>("name")? == name {
                result = Some(json!({
                    "inputEnabled": emission.get::<bool>("input_enabled")?,
                    "directInput": emission.get::<bool>("direct_input")?,
                    "enterPresent": emission.get::<bool>("enter_present")?,
                    "kpEnterPresent": emission.get::<bool>("kp_enter_present")?,
                    "tabPresent": emission.get::<bool>("tab_present")?,
                    "alphaEnabled": emission.get::<bool>("alpha_enabled")?,
                }));
            }
        }
        Ok(result.expect("expected matching script-message emission"))
    }

    fn binding_exists(&self, name: &str) -> mlua::Result<bool> {
        let bindings: Table = self.harness_table()?.get("bindings")?;
        Ok(!matches!(bindings.get::<Value>(name)?, Value::Nil))
    }

    fn binding_key(&self, name: &str) -> mlua::Result<Option<String>> {
        let bindings: Table = self.harness_table()?.get("bindings")?;
        match bindings.get::<Value>(name)? {
            Value::Nil => Ok(None),
            Value::Table(binding) => binding.get("key"),
            value => panic!("unexpected binding value: {value:?}"),
        }
    }

    fn binding_add_count(&self, name: &str) -> mlua::Result<u64> {
        let counts: Table = self.harness_table()?.get("binding_add_counts")?;
        Ok(counts.get::<Option<u64>>(name)?.unwrap_or(0))
    }

    fn section_enabled(&self, name: &str) -> mlua::Result<bool> {
        let sections: Table = self.harness_table()?.get("sections")?;
        match sections.get::<Value>(name)? {
            Value::Nil => Ok(false),
            Value::Table(section) => Ok(section.get::<Option<bool>>("enabled")?.unwrap_or(false)),
            value => panic!("unexpected section value: {value:?}"),
        }
    }

    fn section_define_count(&self, name: &str) -> mlua::Result<u64> {
        let counts: Table = self.harness_table()?.get("section_define_counts")?;
        Ok(counts.get::<Option<u64>>(name)?.unwrap_or(0))
    }

    fn capture_binding(&self, name: &str, capture_name: &str) -> mlua::Result<()> {
        let capture: Function = self.harness_table()?.get("capture_binding")?;
        capture.call((name, capture_name))
    }

    fn invoke_binding(&self, name: &str) -> mlua::Result<()> {
        let invoke: Function = self.harness_table()?.get("invoke_binding")?;
        invoke.call(name)
    }

    fn invoke_captured_binding(&self, capture_name: &str) -> mlua::Result<()> {
        let invoke: Function = self.harness_table()?.get("invoke_captured_binding")?;
        invoke.call(capture_name)
    }

    fn set_time(&self, time: f64) -> mlua::Result<()> {
        self.harness_table()?.set("time", time)
    }

    fn tick(&self) -> mlua::Result<()> {
        let callback: Function = self.harness_table()?.get("periodic_callback")?;
        callback.call(())
    }

    fn option<T: mlua::FromLua>(&self, name: &str) -> mlua::Result<T> {
        let options: Table = self.lua.globals().get("opts")?;
        options.get(name)
    }

    fn input_visible(&self) -> mlua::Result<bool> {
        self.lua.load("return input_ass() ~= ''").eval()
    }

    fn scrolling_item(&self, index: usize) -> mlua::Result<String> {
        let process: Function = self.lua.globals().get("process_chat_item_scrolling")?;
        process.call(index)
    }

    fn process_alert_rows(&self) -> mlua::Result<u64> {
        self.lua
            .load("local rows = process_alert_osd(); return rows")
            .eval()
    }

    fn process_notification_rows(&self) -> mlua::Result<u64> {
        self.lua
            .load("local rows = process_notification_osd(0); return rows")
            .eval()
    }
}

fn settings() -> JsonValue {
    json!({
        "chatInputEnabled": true,
        "chatInputFontFamily": "monospace",
        "chatInputRelativeFontSize": 14,
        "chatInputFontWeight": 1,
        "chatInputFontUnderline": false,
        "chatInputFontColor": "#000000",
        "chatInputPosition": "Top",
        "chatOutputFontFamily": "sans serif",
        "chatOutputRelativeFontSize": 50,
        "chatOutputFontWeight": 1,
        "chatOutputFontUnderline": false,
        "chatOutputMode": "Chatroom",
        "chatMaxLines": 7,
        "chatTopMargin": 25,
        "chatLeftMargin": 20,
        "chatBottomMargin": 30,
        "chatDirectInput": true,
        "notificationTimeout": 3,
        "alertTimeout": 5,
        "chatTimeout": 7,
        "chatOutputEnabled": true,
    })
}

fn statuses(harness: &LuaHarness) -> mlua::Result<Vec<String>> {
    Ok(harness
        .payloads(OPTIONS_ACK_MESSAGE)?
        .into_iter()
        .map(|payload| {
            payload["status"]
                .as_str()
                .expect("ack status should be a string")
                .to_owned()
        })
        .collect())
}

fn table_is_empty(table: Table) -> mlua::Result<bool> {
    Ok(table.pairs::<Value, Value>().next().transpose()?.is_none())
}

#[test]
fn lua_mpv_unique_client_names_leave_only_the_canonical_bridge_active() -> mlua::Result<()> {
    let canonical = LuaHarness::new()?;
    assert_eq!(canonical.payloads(PONG_MESSAGE)?.len(), 1);

    let duplicate = load_script_with_client_name("sorotte_syncplayintf_1")?;
    let duplicate_harness: Table = duplicate.globals().get("__harness")?;
    let duplicate_mp: Table = duplicate.globals().get("mp")?;

    assert!(!duplicate_mp.get::<bool>("keep_running")?);
    assert!(table_is_empty(duplicate_harness.get("messages")?)?);
    assert!(table_is_empty(duplicate_harness.get("bindings")?)?);
    assert!(table_is_empty(duplicate_harness.get("sections")?)?);
    assert!(table_is_empty(duplicate_harness.get("emissions")?)?);
    assert!(matches!(
        duplicate_harness.get::<Value>("periodic_callback")?,
        Value::Nil
    ));
    assert!(matches!(
        duplicate.globals().get::<Value>("opts")?,
        Value::Nil
    ));
    Ok(())
}

#[test]
fn lua_settings_reconfigure_input_and_disable_captured_handlers() -> mlua::Result<()> {
    let harness = LuaHarness::new()?;
    let mut next = settings();
    next["chatInputEnabled"] = json!(false);
    harness.apply("owner-a", "attachment-a", 1, &next)?;

    assert_eq!(statuses(&harness)?, ["applied"]);
    assert_eq!(
        harness.last_emission_snapshot(OPTIONS_ACK_MESSAGE)?,
        json!({
            "inputEnabled": false,
            "directInput": true,
            "enterPresent": false,
            "kpEnterPresent": false,
            "tabPresent": false,
            "alphaEnabled": false,
        }),
        "the acknowledgement must be emitted after disabled state is reconciled"
    );
    assert!(!harness.binding_exists("sorotte-chat-enter")?);
    assert!(!harness.binding_exists("sorotte-chat-kp-enter")?);
    assert!(!harness.binding_exists("sorotte-chat-direct-tab")?);

    next["chatInputEnabled"] = json!(true);
    next["chatDirectInput"] = json!(false);
    harness.apply("owner-a", "attachment-a", 2, &next)?;
    assert_eq!(
        harness.binding_key("sorotte-chat-enter")?,
        Some("enter".to_owned())
    );
    assert_eq!(
        harness.binding_key("sorotte-chat-kp-enter")?,
        Some("kp_enter".to_owned())
    );
    assert!(!harness.binding_exists("sorotte-chat-direct-tab")?);
    assert!(!harness.section_enabled("repl-alpha-input")?);

    harness.trigger_text("type", "from enter")?;
    assert!(harness.input_visible()?);
    harness.invoke_binding("sorotte-chat-enter")?;
    assert_eq!(harness.payloads(CHAT_MESSAGE)?[0]["text"], "from enter");
    assert!(!harness.input_visible()?);

    harness.trigger_text("type", "from keypad")?;
    harness.invoke_binding("sorotte-chat-kp-enter")?;
    assert_eq!(harness.payloads(CHAT_MESSAGE)?[1]["text"], "from keypad");

    harness.trigger_text("type", "must be cleared")?;
    harness.capture_binding("sorotte-chat-enter", "old-enter")?;
    next["chatInputEnabled"] = json!(false);
    harness.apply("owner-a", "attachment-a", 3, &next)?;
    assert!(!harness.input_visible()?);
    assert!(!harness.binding_exists("sorotte-chat-enter")?);
    assert!(!harness.binding_exists("sorotte-chat-kp-enter")?);
    harness.invoke_captured_binding("old-enter")?;
    harness.trigger_text("type", "also ignored")?;
    assert_eq!(harness.payloads(CHAT_MESSAGE)?.len(), 2);
    assert!(!harness.input_visible()?);

    next["chatInputEnabled"] = json!(true);
    next["chatDirectInput"] = json!(true);
    harness.apply("owner-a", "attachment-a", 4, &next)?;
    assert_eq!(
        harness.binding_key("sorotte-chat-direct-tab")?,
        Some("tab".to_owned())
    );
    assert!(harness.section_enabled("repl-alpha-input")?);
    assert_eq!(harness.binding_add_count("sorotte-chat-enter")?, 2);
    assert_eq!(harness.binding_add_count("sorotte-chat-kp-enter")?, 2);
    harness.invoke_binding("sorotte-chat-enter")?;
    harness.invoke_binding("sorotte-chat-enter")?;
    assert_eq!(
        harness.payloads(CHAT_MESSAGE)?.len(),
        2,
        "disabling input must clear the pending REPL text before input is re-enabled"
    );
    Ok(())
}

#[test]
fn lua_direct_input_toggles_idempotently_and_heartbeat_preserves_tab_escape() -> mlua::Result<()> {
    let harness = LuaHarness::new()?;
    let mut next = settings();
    harness.apply("owner-a", "attachment-a", 1, &next)?;
    assert!(harness.section_enabled("repl-alpha-input")?);
    assert_eq!(harness.binding_add_count("__repl_alpha_binding_1")?, 1);
    assert_eq!(harness.section_define_count("repl-alpha-input")?, 1);

    harness.invoke_binding("sorotte-chat-direct-tab")?;
    assert!(!harness.section_enabled("repl-alpha-input")?);
    harness.set_time(1.0)?;
    harness.trigger_json(
        HEARTBEAT_MESSAGE,
        &harness.controller_payload("owner-a", "attachment-a"),
    )?;
    assert!(
        !harness.section_enabled("repl-alpha-input")?,
        "heartbeats must renew only the lease, not undo the user's Tab escape"
    );

    next["chatDirectInput"] = json!(false);
    harness.apply("owner-a", "attachment-a", 2, &next)?;
    assert!(!harness.binding_exists("sorotte-chat-direct-tab")?);
    assert!(!harness.section_enabled("repl-alpha-input")?);

    next["chatDirectInput"] = json!(true);
    harness.apply("owner-a", "attachment-a", 3, &next)?;
    assert_eq!(
        harness.binding_key("sorotte-chat-direct-tab")?,
        Some("tab".to_owned())
    );
    assert!(harness.section_enabled("repl-alpha-input")?);
    assert_eq!(harness.binding_add_count("__repl_alpha_binding_1")?, 1);
    assert_eq!(harness.section_define_count("repl-alpha-input")?, 1);
    assert_eq!(harness.binding_add_count("sorotte-chat-enter")?, 1);
    assert_eq!(harness.binding_add_count("sorotte-chat-kp-enter")?, 1);
    Ok(())
}

#[test]
fn lua_recomputes_scrolling_rows_and_uses_distinct_osd_timeouts() -> mlua::Result<()> {
    let harness = LuaHarness::new()?;
    let mut next = settings();
    next["chatInputEnabled"] = json!(false);
    next["chatOutputMode"] = json!("Scrolling");
    next["notificationTimeout"] = json!(1);
    next["alertTimeout"] = json!(5);
    harness.apply("owner-a", "attachment-a", 1, &next)?;

    harness.trigger_text("chat", "one")?;
    harness.trigger_text("chat", "two")?;
    harness.trigger_text("chat", "three")?;
    assert!(harness.scrolling_item(1)?.contains(",225)"));
    assert!(harness.scrolling_item(2)?.contains(",325)"));
    assert!(harness.scrolling_item(3)?.contains(",425)"));

    next["chatOutputRelativeFontSize"] = json!(100);
    next["chatBottomMargin"] = json!(400);
    harness.apply("owner-a", "attachment-a", 2, &next)?;
    assert!(harness.scrolling_item(1)?.contains(",425)"));
    assert!(harness.scrolling_item(2)?.contains(",425)"));
    assert!(harness.scrolling_item(3)?.contains(",425)"));

    harness.trigger_text("notification-osd-neutral", "short notification")?;
    harness.trigger_text("alert-osd-neutral", "long alert")?;
    assert_eq!(harness.process_notification_rows()?, 1);
    assert_eq!(harness.process_alert_rows()?, 1);
    harness.set_time(1.1)?;
    assert_eq!(harness.process_notification_rows()?, 0);
    assert_eq!(harness.process_alert_rows()?, 1);
    harness.set_time(5.1)?;
    assert_eq!(harness.process_alert_rows()?, 0);
    Ok(())
}

#[test]
fn lua_generation_acknowledgements_are_exact_and_idempotent() -> mlua::Result<()> {
    let harness = LuaHarness::new()?;
    let mut next = settings();
    next["chatDirectInput"] = json!(false);
    harness.apply("owner-a", "attachment-a", 2, &next)?;
    assert_eq!(statuses(&harness)?, ["applied"]);
    assert_eq!(
        harness.payloads(OPTIONS_ACK_MESSAGE)?,
        [json!({
            "protocol": SOROTTE_PROTOCOL,
            "bridgeInstanceId": harness.bridge_instance_id.as_str(),
            "ownerId": "owner-a",
            "attachmentId": "attachment-a",
            "generation": 2,
            "status": "applied",
        })],
        "the applied acknowledgement must echo the exact bridge, controller, and generation"
    );
    assert!(harness.option::<bool>("chatInputEnabled")?);

    let mut duplicate_with_different_values = next.clone();
    duplicate_with_different_values["chatInputEnabled"] = json!(false);
    harness.apply(
        "owner-a",
        "attachment-a",
        2,
        &duplicate_with_different_values,
    )?;
    assert_eq!(statuses(&harness)?, ["applied", "applied"]);
    assert_eq!(
        harness.payloads(OPTIONS_ACK_MESSAGE)?[1],
        json!({
            "protocol": SOROTTE_PROTOCOL,
            "bridgeInstanceId": harness.bridge_instance_id.as_str(),
            "ownerId": "owner-a",
            "attachmentId": "attachment-a",
            "generation": 2,
            "status": "applied",
        }),
        "an idempotent duplicate must re-acknowledge the exact original generation"
    );
    assert!(
        harness.option::<bool>("chatInputEnabled")?,
        "a duplicate generation must be acknowledged without reapplying changed values"
    );

    harness.apply("owner-a", "attachment-a", 1, &next)?;
    assert_eq!(statuses(&harness)?, ["applied", "applied", "rejected"]);
    assert_eq!(
        harness.payloads(OPTIONS_ACK_MESSAGE)?[2],
        json!({
            "protocol": SOROTTE_PROTOCOL,
            "bridgeInstanceId": harness.bridge_instance_id.as_str(),
            "ownerId": "owner-a",
            "attachmentId": "attachment-a",
            "generation": 1,
            "status": "rejected",
            "error": "stale settings generation",
        }),
        "a stale acknowledgement must identify the rejected generation exactly"
    );
    assert!(harness.option::<bool>("chatInputEnabled")?);

    let ack_count = statuses(&harness)?.len();
    let mut malformed = next.clone();
    malformed
        .as_object_mut()
        .expect("settings should be an object")
        .remove("chatTopMargin");
    harness.apply("owner-a", "attachment-a", 3, &malformed)?;
    assert_eq!(statuses(&harness)?.len(), ack_count);
    assert_eq!(harness.option::<i64>("chatTopMargin")?, 25);

    let wrong_bridge = json!({
        "protocol": SOROTTE_PROTOCOL,
        "bridgeInstanceId": "not-this-bridge",
        "ownerId": "owner-a",
        "attachmentId": "attachment-a",
        "generation": 3,
        "leaseMs": 2_000,
        "settings": next,
    });
    harness.trigger_json(OPTIONS_MESSAGE, &wrong_bridge)?;
    assert_eq!(statuses(&harness)?.len(), ack_count);
    Ok(())
}

#[test]
fn lua_disabled_input_configuration_does_not_hold_or_reacquire_a_lease() -> mlua::Result<()> {
    let harness = LuaHarness::new()?;
    let mut next = settings();
    next["chatInputEnabled"] = json!(false);
    harness.apply("owner-a", "attachment-a", 1, &next)?;
    assert_eq!(statuses(&harness)?, ["applied"]);
    assert!(!harness.binding_exists("sorotte-chat-enter")?);

    harness.trigger_json(
        PING_MESSAGE,
        &json!({"protocol": SOROTTE_PROTOCOL, "nonce": "disabled-owner-check"}),
    )?;
    let pong = harness
        .payloads(PONG_MESSAGE)?
        .pop()
        .expect("the disabled bridge should still answer discovery");
    assert!(pong.get("activeOwnerId").is_none());
    assert!(pong.get("activeAttachmentId").is_none());
    assert_eq!(pong["leaseRemainingMs"], 0);

    harness.set_time(10.0)?;
    harness.tick()?;
    assert!(harness.payloads(LEASE_EXPIRED_MESSAGE)?.is_empty());
    assert!(!harness.binding_exists("sorotte-chat-enter")?);
    Ok(())
}

#[test]
fn lua_lease_expiry_release_and_new_owner_takeover_are_safe() -> mlua::Result<()> {
    let harness = LuaHarness::new()?;
    let owner_a_settings = settings();
    let mut owner_b_settings = settings();
    owner_b_settings["chatDirectInput"] = json!(false);
    owner_b_settings["chatOutputRelativeFontSize"] = json!(80);
    owner_b_settings["chatTopMargin"] = json!(111);
    owner_b_settings["notificationTimeout"] = json!(9);
    harness.apply("owner-a", "attachment-a", 1, &owner_a_settings)?;
    harness.capture_binding("sorotte-chat-enter", "leased-enter")?;

    harness.set_time(0.5)?;
    harness.apply("owner-b", "attachment-b", 1, &owner_b_settings)?;
    assert_eq!(statuses(&harness)?, ["applied", "busy"]);
    assert_eq!(
        harness.payloads(OPTIONS_ACK_MESSAGE)?[1],
        json!({
            "protocol": SOROTTE_PROTOCOL,
            "bridgeInstanceId": harness.bridge_instance_id.as_str(),
            "ownerId": "owner-b",
            "attachmentId": "attachment-b",
            "generation": 1,
            "status": "busy",
            "error": "another Sorotte owner holds the live bridge lease",
        })
    );
    assert!(harness.option::<bool>("chatDirectInput")?);
    assert_eq!(harness.option::<i64>("chatOutputRelativeFontSize")?, 50);
    assert_eq!(harness.option::<i64>("chatTopMargin")?, 25);
    assert_eq!(harness.option::<i64>("notificationTimeout")?, 3);
    assert!(harness.binding_exists("sorotte-chat-enter")?);
    assert!(harness.binding_exists("sorotte-chat-direct-tab")?);
    harness.trigger_json(
        PING_MESSAGE,
        &json!({"protocol": SOROTTE_PROTOCOL, "nonce": "busy-owner-check"}),
    )?;
    let pong = harness
        .payloads(PONG_MESSAGE)?
        .pop()
        .expect("the bridge should answer the ownership probe");
    assert_eq!(pong["activeOwnerId"], "owner-a");
    assert_eq!(pong["activeAttachmentId"], "attachment-a");

    harness.trigger_json(
        RELEASE_MESSAGE,
        &harness.controller_payload("owner-a", "stale-attachment"),
    )?;
    assert!(harness.binding_exists("sorotte-chat-enter")?);

    harness.invoke_binding("sorotte-chat-direct-tab")?;
    assert!(!harness.section_enabled("repl-alpha-input")?);
    harness.set_time(1.0)?;
    harness.trigger_json(
        HEARTBEAT_MESSAGE,
        &harness.controller_payload("owner-a", "attachment-a"),
    )?;
    assert!(!harness.section_enabled("repl-alpha-input")?);

    harness.set_time(2.9)?;
    harness.tick()?;
    assert!(harness.binding_exists("sorotte-chat-enter")?);
    harness.set_time(3.1)?;
    harness.tick()?;
    assert!(!harness.binding_exists("sorotte-chat-enter")?);
    assert_eq!(
        harness.payloads(LEASE_EXPIRED_MESSAGE)?,
        [json!({
            "protocol": SOROTTE_PROTOCOL,
            "bridgeInstanceId": harness.bridge_instance_id.as_str(),
            "ownerId": "owner-a",
            "attachmentId": "attachment-a",
        })],
        "lease expiry must identify exactly the controller whose input was deactivated"
    );
    harness.invoke_captured_binding("leased-enter")?;
    assert!(harness.payloads(CHAT_MESSAGE)?.is_empty());

    harness.apply("owner-b", "attachment-b", 1, &owner_b_settings)?;
    assert_eq!(statuses(&harness)?, ["applied", "busy", "applied"]);
    assert!(harness.binding_exists("sorotte-chat-enter")?);
    assert!(!harness.binding_exists("sorotte-chat-direct-tab")?);
    assert!(!harness.section_enabled("repl-alpha-input")?);
    assert!(!harness.option::<bool>("chatDirectInput")?);
    assert_eq!(harness.option::<i64>("chatOutputRelativeFontSize")?, 80);
    assert_eq!(harness.option::<i64>("chatTopMargin")?, 111);
    assert_eq!(harness.option::<i64>("notificationTimeout")?, 9);

    harness.trigger_json(
        RELEASE_MESSAGE,
        &harness.controller_payload("owner-a", "attachment-a"),
    )?;
    assert!(harness.binding_exists("sorotte-chat-enter")?);
    harness.trigger_json(
        RELEASE_MESSAGE,
        &harness.controller_payload("owner-b", "attachment-b"),
    )?;
    assert!(!harness.binding_exists("sorotte-chat-enter")?);
    assert!(!harness.section_enabled("repl-alpha-input")?);
    Ok(())
}
