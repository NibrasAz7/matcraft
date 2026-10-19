//! matcraft-cli — MatCraft from the command line.
//!
//!   matcraft-cli run <file.m> [--cmd 'id {json}']…   run a script, then commands
//!   matcraft-cli eval <code> [--cmd 'id {json}']…    evaluate, then commands
//!   matcraft-cli repl                                interactive prompt
//!   matcraft-cli functions                           list builtin functions
//!   matcraft-cli commands                            list engine commands
//!   matcraft-cli mcp                                 MCP server on stdio
//!   matcraft-cli --version
//!
//! `--cmd` follows the sibling apps' convention: `'id {json}'`, repeatable,
//! each reply printed as one JSON line.

use std::io::{BufRead, Write};
use std::process::ExitCode;

use matcraft_engine::Session;
use serde_json::{json, Value as Json};

const USAGE: &str = "\
matcraft-cli — headless MatCraft

USAGE:
  matcraft-cli run <file.m> [--cmd 'id {json}']…   run an M-code script
  matcraft-cli eval <code> [--cmd 'id {json}']…    evaluate code
  matcraft-cli repl                                interactive read-eval-print loop
  matcraft-cli functions                           list the builtin function library
  matcraft-cli commands                            list engine commands
  matcraft-cli mcp                                 serve MCP (JSON-RPC on stdio)
  matcraft-cli --version                           print version

--cmd takes `'id {json}'` (id plus a space and a JSON object) and is
repeatable; each command prints its JSON result on its own line.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("run") | Some("eval") => {
            let sub = args[0].as_str();
            let (input, cmds) = match split_cmds(&args[1..]) {
                Ok(v) => v,
                Err(e) => return fail(&e),
            };
            let Some(first) = input.first() else {
                return fail(&format!("{sub} needs {} and optionally --cmd", if sub == "run" { "a file path" } else { "code" }));
            };
            if sub == "run" {
                match matlang::read_m_source(std::path::Path::new(first)) {
                    Ok(src) => {
                        let mut session = Session::new();
                        // The script's own directory is on the function path.
                        if let Some(d) = std::path::Path::new(first).parent() {
                            session.lang_mut().add_path(d.to_path_buf());
                        }
                        match session.lang_mut().run(&src) {
                            Ok(out) => print!("{out}"),
                            Err(e) => return fail(&e.to_string()),
                        }
                        run_cmds(&mut session, &cmds)
                    }
                    Err(e) => fail(&format!("cannot read {first}: {e}")),
                }
            } else {
                let mut session = Session::new();
                match session.lang_mut().run(first) {
                    Ok(out) => print!("{out}"),
                    Err(e) => return fail(&e.to_string()),
                }
                run_cmds(&mut session, &cmds)
            }
        }
        Some("repl") => repl(),
        Some("functions") => {
            for (group, names) in matcraft_engine::cmd::FUNCTION_GROUPS {
                println!("{group:>16}:  {names}");
            }
            ExitCode::SUCCESS
        }
        Some("commands") => {
            let session = Session::new();
            for c in session.commands() {
                println!("{:>18}  {:<42} {params}", c.id, c.label, params = c.params);
            }
            ExitCode::SUCCESS
        }
        Some("mcp") => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut server = matcraft_mcp::Server::new();
            match server.serve(stdin.lock(), stdout.lock()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&format!("mcp: {e}")),
            }
        }
        Some("--version") | Some("-V") => {
            println!("matcraft-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") | None => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => fail(&format!("unknown subcommand '{other}'\n\n{USAGE}")),
    }
}

/// A `--cmd` invocation: command id plus JSON params.
type Cmd = (String, Json);

/// Split trailing `--cmd 'id {json}'` flags from the positional input.
fn split_cmds(args: &[String]) -> Result<(Vec<String>, Vec<Cmd>), String> {
    let mut input = Vec::new();
    let mut cmds = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--cmd" {
            let raw = args.get(i + 1).ok_or("--cmd needs a value: 'id {json}'")?;
            let (id, rest) = raw.split_once(char::is_whitespace).unwrap_or((raw.as_str(), "{}"));
            let params: Json = serde_json::from_str(rest.trim()).map_err(|e| format!("--cmd '{id}': bad JSON ({e})"))?;
            cmds.push((id.to_string(), params));
            i += 2;
        } else {
            input.push(args[i].clone());
            i += 1;
        }
    }
    Ok((input, cmds))
}

fn run_cmds(session: &mut Session, cmds: &[Cmd]) -> ExitCode {
    for (id, params) in cmds {
        match session.execute(id, params.clone()) {
            Ok(result) => println!("{}", json!({"ok": true, "command": id, "result": result})),
            Err(e) => {
                println!("{}", json!({"ok": false, "command": id, "error": e.to_string()}));
                return fail(&format!("command '{id}' failed: {e}"));
            }
        }
    }
    ExitCode::SUCCESS
}

fn repl() -> ExitCode {
    let stdin = std::io::stdin();
    let mut session = Session::new();
    eprintln!("MatCraft {} — Ctrl-D to exit", env!("CARGO_PKG_VERSION"));
    loop {
        eprint!("matcraft> ");
        if std::io::stderr().flush().is_err() {
            return ExitCode::FAILURE;
        }
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => return ExitCode::SUCCESS, // EOF
            Ok(_) => {}
            Err(e) => return fail(&format!("input error: {e}")),
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match session.lang_mut().run(trimmed) {
            Ok(out) => print!("{out}"),
            Err(e) => eprintln!("{e}"),
        }
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::FAILURE
}
