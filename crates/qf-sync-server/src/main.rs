use qf_sync_server::Database;
use std::path::Path;

const USAGE: &str = "\
qf-sync-server init DATABASE
qf-sync-server serve DATABASE [ADDRESS]       (default 127.0.0.1:8787)
qf-sync-server enroll DATABASE DEVICE_LABEL   (prints the new token once)
qf-sync-server rotate DATABASE DEVICE_UUID    (same device; new token)
qf-sync-server revoke DATABASE DEVICE_UUID
qf-sync-server backup DATABASE NEW_BACKUP_FILE
qf-sync-server restore-generation DATABASE    (service must be stopped)

Use a dedicated private data directory. init never overwrites an existing DB.
Serve behind HTTPS. A restored backup must receive a new generation before use.";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("qf-sync-server: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "help" {
        println!("{USAGE}");
        return Ok(());
    }
    let path = args.get(1).ok_or(USAGE)?;
    match args[0].as_str() {
        "init" if args.len() == 2 => {
            Database::create(Path::new(path))?;
            println!("Created an empty sync database. Enroll a device to initialize its queue.");
        }
        "serve" if (2..=3).contains(&args.len()) => {
            let database = Database::open(Path::new(path))?;
            let address = args.get(2).map(String::as_str).unwrap_or("127.0.0.1:8787");
            let listener = tokio::net::TcpListener::bind(address).await?;
            eprintln!("queue sync listening on {}", listener.local_addr()?);
            axum::serve(listener, qf_sync_server::router(database))
                .with_graceful_shutdown(shutdown())
                .await?;
        }
        "enroll" | "rotate" if args.len() == 3 => {
            let mut database = Database::open(Path::new(path))?;
            let enrollment = if args[0] == "enroll" {
                database.enroll(&args[2])?
            } else {
                database.rotate(args[2].parse()?)?
            };
            println!("Device: {}\nToken: {}", enrollment.device, enrollment.token);
        }
        "revoke" if args.len() == 3 => {
            Database::open(Path::new(path))?.revoke(args[2].parse()?)?;
            println!("Device revoked.");
        }
        "backup" if args.len() == 3 => {
            Database::open(Path::new(path))?.backup(Path::new(&args[2]))?;
            println!("Consistent backup completed.");
        }
        "restore-generation" if args.len() == 2 => {
            Database::open(Path::new(path))?.restore_generation()?;
            println!("Restored history has a new generation. Clients will enter recovery.");
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

async fn shutdown() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = terminate.recv() => {},
    }
}
