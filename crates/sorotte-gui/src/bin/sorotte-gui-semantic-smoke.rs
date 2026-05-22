fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.is_empty() {
        sorotte_gui::semantic_smoke::run_sorotte_gui_semantic_cli_from_env()
    } else {
        sorotte_gui::semantic_smoke::run_sorotte_gui_semantic_cli_from_args(
            args.iter().map(String::as_str),
        )
    };

    match result {
        Ok(Some(output)) => {
            print!("{output}");
        }
        Ok(None) => {
            eprintln!(
                "sorotte-gui-semantic-smoke requires --scenario NAME, --script PATH, \
--append-script PATH (with --scenario NAME), \
--print-script NAME, \
--describe-scenarios, \
--inline-script TEXT, or SOROTTE_GUI_SEMANTIC_SCENARIO / \
SOROTTE_GUI_SEMANTIC_SCENARIO_PATH"
            );
            std::process::exit(2);
        }
        Err(error) => {
            eprintln!("sorotte-gui-semantic-smoke failed: {error}");
            std::process::exit(1);
        }
    }
}
