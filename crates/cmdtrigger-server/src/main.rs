//! `cmdtrigger-server`: serve, or manage accounts.
//!
//! ```text
//! cmdtrigger-server                         serve (CMDTRIGGER_ADDR, CMDTRIGGER_DB, CMDTRIGGER_ALLOW_SIGNUP)
//! cmdtrigger-server list-users
//! cmdtrigger-server delete-user EMAIL
//! cmdtrigger-server reset-password EMAIL    (asks for the new password)
//! ```
//!
//! `CMDTRIGGER_ALLOW_SIGNUP`: unset = only the first account (yours), `true` = anyone,
//! `false` = nobody.

use cmdtrigger_server::{Server, Signup, router};
use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .init();
    let db = PathBuf::from(std::env::var("CMDTRIGGER_DB").unwrap_or_else(|_| "cmdtrigger-server.db".into()));
    let signup = Signup::from_env(std::env::var("CMDTRIGGER_ALLOW_SIGNUP").ok().as_deref());
    let server = match Server::open(&db, signup) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cmdtrigger-server: can't open {}: {e}", db.display());
            return ExitCode::FAILURE;
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["serve"] => {
            let addr: SocketAddr =
                match std::env::var("CMDTRIGGER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into()).parse() {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("CMDTRIGGER_ADDR: {e}");
                        return ExitCode::FAILURE;
                    }
                };
            let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("can't listen on {addr}: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                tracing::info!("CmdTrigger server on http://{addr} (database {}, signup {:?})", db.display(), signup);
                let shutdown = async {
                    let _ = tokio::signal::ctrl_c().await;
                };
                let _ = axum::serve(listener, router(server)).with_graceful_shutdown(shutdown).await;
                ExitCode::SUCCESS
            })
        }
        ["list-users"] => match server.list_users() {
            Ok(users) => {
                for (email, created, computers) in users {
                    println!("{email}\tcreated {created}\t{computers} computer(s)");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        ["delete-user", email] => match server.delete_user(email) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => {
                eprintln!("no account {email}");
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        ["reset-password", email] => {
            eprint!("New password for {email}: ");
            let mut line = String::new();
            let _ = std::io::stdin().lock().read_line(&mut line);
            let password = line.trim();
            if password.chars().count() < 8 {
                eprintln!("use at least 8 characters");
                return ExitCode::FAILURE;
            }
            match server.reset_password(email, password) {
                Ok(true) => ExitCode::SUCCESS,
                _ => {
                    eprintln!("no account {email}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: cmdtrigger-server [serve | list-users | delete-user EMAIL | reset-password EMAIL]");
            ExitCode::FAILURE
        }
    }
}
