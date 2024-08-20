use anyhow::Result;
use baml_runtime::{BamlRuntime, CallerType};

fn main() -> Result<()> {
    if let Err(e) = env_logger::try_init_from_env(
        env_logger::Env::new()
            .filter("BAML_LOG")
            .write_style("BAML_LOG_STYLE"),
    ) {
        eprintln!("Failed to initialize BAML logger: {:#}", e);
    };

    println!("Hello from the binary reuilb!");

    let argv: Vec<String> = std::env::args().collect();

    BamlRuntime::run_cli(argv, CallerType::Python)
}
