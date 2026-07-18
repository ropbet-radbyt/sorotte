use mlua::{Function, Lua, LuaSerdeExt, Table, Value};
use serde_json::{Value as JsonValue, json};

const PROTOCOL: &str = "sorotte-network-options-v1";
const CONFIGURE_MESSAGE: &str = "sorotte_network_options_configure";
const APPLY_ACTIVE_MESSAGE: &str = "sorotte_network_options_apply_active";
const SCRIPT_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/sorotte_network_options.lua"
));

const MP_MOCK_SOURCE: &str = r#"
__harness = {
    messages = {}, hooks = {}, properties = {}, writes = {}, emissions = {}, reject = nil,
}
mp = { keep_running = true }
function mp.get_script_name() return __script_name end
function mp.register_script_message(name, callback) __harness.messages[name] = callback end
function mp.add_hook(name, priority, callback) __harness.hooks[name] = callback end
function mp.get_property(name, default)
    local value = __harness.properties[name]
    if value == nil then return default end
    return value
end
function mp.set_property(name, value)
    table.insert(__harness.writes, { name = name, value = value, path = __harness.properties.path })
    if __harness.reject == name then return nil, 'rejected ' .. name end
    return true
end
function mp.commandv(...)
    local args = {...}
    if args[1] == 'script-message' then
        table.insert(__harness.emissions, { name = args[2], payload = args[3] })
    end
end
package.preload['mp.utils'] = function()
    return { parse_json = __parse_json, format_json = __format_json }
end
"#;

struct Harness {
    lua: Lua,
}

impl Harness {
    fn new() -> mlua::Result<Self> {
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
        lua.globals()
            .set("__script_name", "sorotte_network_options")?;
        lua.load(MP_MOCK_SOURCE).exec()?;
        lua.load(SCRIPT_SOURCE).exec()?;
        Ok(Self { lua })
    }

    fn table(&self) -> mlua::Result<Table> {
        self.lua.globals().get("__harness")
    }

    fn send(&self, name: &str, payload: JsonValue) -> mlua::Result<()> {
        let messages: Table = self.table()?.get("messages")?;
        let callback: Function = messages.get(name)?;
        callback.call(serde_json::to_string(&payload).unwrap())
    }

    fn configure(&self) -> mlua::Result<()> {
        self.send(
            CONFIGURE_MESSAGE,
            json!({
                "protocol": PROTOCOL,
                "attachment": "harness-attachment",
                "generation": 7,
                "options": {"cache-secs": "75", "cache-pause-wait": "5"},
            }),
        )
    }

    fn set_path(&self, property: &str, path: &str) -> mlua::Result<()> {
        let properties: Table = self.table()?.get("properties")?;
        properties.set(property, path)
    }

    fn invoke_on_load(&self) -> mlua::Result<()> {
        let hooks: Table = self.table()?.get("hooks")?;
        let callback: Function = hooks.get("on_load")?;
        callback.call(())
    }

    fn writes(&self) -> mlua::Result<Vec<(String, String, String)>> {
        let writes: Table = self.table()?.get("writes")?;
        writes
            .sequence_values::<Table>()
            .map(|write| {
                let write = write?;
                Ok((write.get("name")?, write.get("value")?, write.get("path")?))
            })
            .collect()
    }

    fn emissions(&self, name: &str) -> mlua::Result<Vec<JsonValue>> {
        let emissions: Table = self.table()?.get("emissions")?;
        emissions
            .sequence_values::<Table>()
            .filter_map(|emission| match emission {
                Ok(emission) if emission.get::<String>("name").ok().as_deref() == Some(name) => {
                    Some(emission.get::<String>("payload").map(|payload| {
                        serde_json::from_str(&payload).expect("hook payload should be valid JSON")
                    }))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
}

#[test]
fn on_load_applies_the_complete_map_only_to_the_path_sampled_inside_mpv() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure()?;
    harness.set_path("path", "https://media.example.test/a.m3u8")?;
    harness.set_path("stream-open-filename", "https://media.example.test/a.m3u8")?;
    harness.invoke_on_load()?;

    let mut writes = harness.writes()?;
    writes.sort_unstable();
    assert_eq!(
        writes,
        [
            (
                "file-local-options/cache-pause-wait".to_owned(),
                "5".to_owned(),
                "https://media.example.test/a.m3u8".to_owned(),
            ),
            (
                "file-local-options/cache-secs".to_owned(),
                "75".to_owned(),
                "https://media.example.test/a.m3u8".to_owned(),
            ),
        ]
    );
    assert_eq!(
        harness.emissions("sorotte-network-options-transition-result")?[0]["status"],
        "network-updated"
    );

    harness.set_path("path", "C:/media/local.mkv")?;
    harness.set_path("stream-open-filename", "C:/media/local.mkv")?;
    harness.invoke_on_load()?;
    assert_eq!(
        harness.writes()?.len(),
        2,
        "a later local on-load callback must not inherit any network-only write"
    );
    Ok(())
}

#[test]
fn explicit_apply_reports_the_authoritative_path_used_for_its_atomic_write_set() -> mlua::Result<()>
{
    let harness = Harness::new()?;
    harness.configure()?;
    harness.set_path("path", "https://media.example.test/b.m3u8")?;
    harness.send(
        APPLY_ACTIVE_MESSAGE,
        json!({
            "protocol": PROTOCOL,
            "attachment": "harness-attachment",
            "generation": 7,
            "attempt": 42,
        }),
    )?;

    let result = &harness.emissions("sorotte-network-options-active-result")?[0];
    assert_eq!(result["attempt"], 42);
    assert_eq!(result["path"], "https://media.example.test/b.m3u8");
    assert_eq!(result["status"], "network-updated");
    assert!(
        harness
            .writes()?
            .iter()
            .all(|(_, _, path)| path == "https://media.example.test/b.m3u8")
    );
    Ok(())
}
