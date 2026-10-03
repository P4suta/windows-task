use std::{env, fs, io::Write, path::{Path, PathBuf}, process::{Command, ExitCode}};

fn journal(root: &Path, name: &str, entry: &str) -> std::io::Result<()> {
    writeln!(fs::OpenOptions::new().append(true).open(root.join(name))?, "{entry}")
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::var_os("RELEASE_TEST_STATE").ok_or("missing fixture state")?);
    let args: Vec<String> = env::args().skip(1).collect();
    if env::current_exe()?.file_stem() == Some(std::ffi::OsStr::new("cargo")) {
        if !args.starts_with(&["+1.85.0".into(), "xtask".into(), "upload-draft".into()]) {
            return Err("unsupported fixture cargo command".into());
        }
        return Ok(Command::new(env::var_os("RELEASE_XTASK_BINARY").ok_or("missing xtask binary")?)
            .arg("upload-draft").args(&args[3..]).status()?.success());
    }
    let state = fs::read_to_string(root.join("state"))?;
    if args.first().is_some_and(|arg| arg == "api") {
        if state == "unavailable" || root.join("metadata-unavailable").exists() {
            return Ok(false);
        }
        let endpoint = args.iter().find(|arg| arg.starts_with("repos/")).ok_or("missing API endpoint")?;
        if endpoint.ends_with("/releases/tags/v0.1.0") {
            journal(&root, "requests", "metadata")?;
            let mut records = Vec::new();
            for name in ["fixture.zip", "fixture.zip.sha256"] {
                if root.join("assets").join(name).is_file() {
                    records.push(fs::read_to_string(root.join(format!("{name}.json")))?);
                }
            }
            println!("{{\"draft\":{},\"tag_name\":\"v0.1.0\",\"assets\":[{}]}}",
                state.starts_with("draft"), records.join(","));
            return Ok(true);
        }
        let (id, name) = if endpoint.ends_with("/releases/assets/1") {
            (1, "fixture.zip")
        } else if endpoint.ends_with("/releases/assets/2") {
            (2, "fixture.zip.sha256")
        } else {
            return Err("unsupported API endpoint".into());
        };
        journal(&root, "requests", &format!("download:{id}"))?;
        std::io::stdout().write_all(&fs::read(root.join("assets").join(name))?)?;
        return Ok(true);
    }
    let [release, operation, _rest @ ..] = args.as_slice() else {
        return Err("missing fixture command".into());
    };
    if release != "release" {
        return Err("unsupported fixture command".into());
    }
    journal(&root, "requests", operation)?;
    if state == "unavailable" {
        return Ok(false);
    }
    match operation.as_str() {
        "view" => {
            if state == "missing" { return Ok(false); }
            if args.iter().any(|arg| arg == "--json") {
                println!("{{\"isDraft\":{}}}", state.starts_with("draft"));
            }
            Ok(true)
        }
        "create" | "upload" => {
            if (operation == "create" && state != "missing") || state == "published" {
                return Ok(false);
            }
            if args.iter().any(|arg| arg == "--clobber") {
                return Err("asset replacement is forbidden".into());
            }
            let files: Vec<_> = args.iter().map(Path::new).filter(|path| path.is_file()).collect();
            if files.is_empty() { return Ok(false); }
            if operation == "create" && (!args.iter().any(|arg| arg == "--draft") || files.len() != 2) {
                return Ok(false);
            }
            for path in files {
                let name = path.file_name().ok_or("asset name")?.to_str().ok_or("UTF-8 asset name")?;
                let target = root.join("assets").join(name);
                if target.exists() { return Ok(false); }
                let fail = root.join("fail-once");
                if fail.exists() && fs::read_to_string(&fail)? == name {
                    fs::remove_file(fail)?;
                    return Ok(false);
                }
                fs::copy(path, target)?;
                if operation == "upload" {
                    journal(&root, "effects", &format!("upload:{name}"))?;
                }
            }
            if operation == "create" {
                journal(&root, "effects", "create-draft")?;
                fs::write(root.join("state"), "draft")?;
            }
            Ok(true)
        }
        "edit" if state.starts_with("draft") => {
            let verifies_tag = args.iter().any(|arg| arg == "--verify-tag")
                && args.windows(2).any(|pair| pair == ["--tag", "v0.1.0"]);
            if verifies_tag && state == "draft-tag-missing" { return Ok(false); }
            if args.iter().any(|arg| arg == "--draft=false") {
                journal(&root, "effects", "publish")?;
                fs::write(root.join("state"), "published")?;
                return Ok(true);
            }
            Ok(false)
        }
        _ => Err("unsupported fixture transition".into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => { eprintln!("fixture: {error}"); ExitCode::from(2) }
    }
}
