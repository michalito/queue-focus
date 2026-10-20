use qf_core::sync::*;
use qf_sync::{
    setup::{self, Mode},
    Config,
};
use serde::Deserialize;
use std::path::Path;

const USAGE: &str = "\
Quit Queue Focus before setup, resume, credentials, retry, dismiss or export.
qf-sync preview DIR SERVER_ORIGIN DEVICE_UUID TOKEN_JSON
qf-sync setup DIR SERVER_ORIGIN DEVICE_UUID TOKEN_JSON initialize|import|shared
qf-sync resume DIR
qf-sync status DIR
qf-sync credentials DIR TOKEN_JSON
qf-sync retry DIR
qf-sync dismiss DIR COMMAND_UUID
qf-sync export DIR NEW_STANDALONE_DIR

TOKEN_JSON is a private file containing {\"token\":\"...\"}; never pass a token
as a command-line argument. HTTPS is required. For local testing only, append
--allow-loopback-http to preview or setup with a literal loopback IP address.
initialize uses the first device's queue and timer; import appends local tasks
(local Now becomes Next); shared keeps a backup and uses only the shared queue.";

fn main() {
    if let Err(error) = run() {
        eprintln!("qf-sync: {error}");
        std::process::exit(1);
    }
}
fn run() -> setup::Result<()> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "help" {
        println!("{USAGE}");
        return Ok(());
    }
    let loopback = args.last().is_some_and(|s| s == "--allow-loopback-http");
    if loopback {
        args.pop();
    }
    if args.is_empty() {
        return Err(USAGE.into());
    }
    if loopback && !matches!(args[0].as_str(), "preview" | "setup") {
        return Err(USAGE.into());
    }
    let directory = Path::new(args.get(1).ok_or(USAGE)?);
    match args[0].as_str() {
        "preview" | "setup" if args.len() == if args[0] == "preview" { 5 } else { 6 } => {
            let config = Config {
                version: PROTOCOL_VERSION,
                url: args[2].clone(),
                device: args[3].parse()?,
                token: token(Path::new(&args[4]))?,
                allow_loopback_http: loopback,
            };
            if args[0] == "preview" {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&setup::preview(directory, config)?)?
                );
            } else {
                let mode = match args[5].as_str() {
                    "initialize" => Mode::Initialize,
                    "import" => Mode::Import,
                    "shared" => Mode::Shared,
                    _ => return Err(USAGE.into()),
                };
                setup::setup(directory, config, mode)?;
                println!("Sync activated. Local backup retained. Start Queue Focus to connect.");
            }
        }
        "resume" if args.len() == 2 => {
            setup::resume(directory)?;
            println!("Setup completed. Start Queue Focus to connect.");
        }
        "status" if args.len() == 2 => {
            if directory.join(setup::SETUP_FILE).exists() {
                println!("Setup is incomplete. Quit the app, then run qf-sync resume or export.");
            } else {
                let envelope: Envelope =
                    read_private(&directory.join(STATE_FILE), MAX_ENVELOPE_BYTES)?;
                envelope.validate()?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "device": envelope.device, "dataset": envelope.confirmed.dataset,
                        "revision": envelope.confirmed.revision, "pending": envelope.pending,
                        "recovery": envelope.recovery, "suspension": envelope.suspension,
                        "note": "Connection status belongs to the running app; this is the durable journal."
                    }))?
                );
            }
        }
        "credentials" | "retry" | "dismiss"
            if args.len() == if args[0] == "retry" { 2 } else { 3 } =>
        {
            let _lock = DataLock::acquire(directory)?;
            let mut config = Config::load(directory)?;
            if args[0] == "credentials" {
                config.token = token(Path::new(&args[2]))?;
                config.save(directory)?;
            }
            if directory.join(setup::SETUP_FILE).exists() {
                if args[0] != "credentials" {
                    return Err("setup is incomplete; resume or export it".into());
                }
            } else {
                let mut client = Client::open(&directory.join(STATE_FILE), config.device)?;
                if args[0] == "dismiss" {
                    client.dismiss_recovery(args[2].parse()?)?;
                } else {
                    client.resume_after_repair()?;
                }
            }
            println!("Saved. Start Queue Focus to continue syncing.");
        }
        "export" if args.len() == 3 => {
            setup::export(directory, Path::new(&args[2]))?;
            println!("Exported standalone tasks and a separate intent record. Source installation retained.");
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
fn token(path: &Path) -> setup::Result<String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Secret {
        token: String,
    }
    Ok(read_private::<Secret>(path, 4096)?.token)
}
