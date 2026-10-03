use std::{env, fs, io::Write, path::PathBuf, process::ExitCode};

#[derive(Clone, Copy)]
enum State {
    Missing,
    Draft,
    Published,
    Unavailable,
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::var_os("RELEASE_TEST_STATE").ok_or("missing fixture state")?);
    let state = match fs::read_to_string(root.join("state"))?.trim() {
        "missing" => State::Missing,
        "draft" => State::Draft,
        "published" => State::Published,
        "unavailable" => State::Unavailable,
        _ => return Err("unknown fixture state".into()),
    };
    let args: Vec<String> = env::args().skip(1).collect();
    let [release, operation, _rest @ ..] = args.as_slice() else {
        return Err("missing fixture command".into());
    };
    if release != "release" {
        return Err("unsupported fixture command".into());
    }
    let request = match operation.as_str() {
        "view" => "view",
        "create" => "create",
        "upload" => "upload",
        "edit" => "edit",
        _ => return Err("unsupported fixture operation".into()),
    };
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(root.join("requests"))?,
        "{request}"
    )?;
    let mut effects = fs::read_to_string(root.join("effects"))?;
    let has_asset = |name: &str| {
        args.iter().any(|arg| {
            let path = std::path::Path::new(arg);
            path.file_name() == Some(std::ffi::OsStr::new(name)) && path.is_file()
        })
    };
    let complete_assets = has_asset("fixture.zip") && has_asset("fixture.zip.sha256");
    let accepted = match (operation.as_str(), state) {
        ("view", State::Draft | State::Published) => {
            if args.iter().any(|arg| arg == "--json") {
                println!("{{\"isDraft\":{}}}", matches!(state, State::Draft));
            }
            true
        }
        ("view" | "create" | "upload" | "edit", State::Unavailable) => false,
        ("view", State::Missing) => false,
        ("create", State::Missing) => {
            if !complete_assets {
                return Ok(false);
            }
            if args.iter().any(|arg| arg == "--draft") {
                effects.push_str("create-draft\n");
                fs::write(root.join("state"), "draft")?;
            } else {
                effects.push_str("publish-incomplete\n");
                fs::write(root.join("state"), "published")?;
            }
            true
        }
        ("create", State::Draft | State::Published) => false,
        ("upload", State::Draft) => {
            if !complete_assets {
                return Ok(false);
            }
            if args.iter().any(|arg| arg == "--clobber") {
                effects.push_str("replace-existing\n");
            }
            effects.push_str("upload\n");
            true
        }
        ("upload", State::Published) => {
            effects.push_str("modify-published\n");
            true
        }
        ("edit", State::Draft) if args.iter().any(|arg| arg == "--draft=false") => {
            effects.push_str("publish\n");
            fs::write(root.join("state"), "published")?;
            true
        }
        _ => return Err("unsupported fixture transition".into()),
    };
    fs::write(root.join("effects"), effects)?;
    Ok(accepted)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(_) => ExitCode::from(2),
    }
}
