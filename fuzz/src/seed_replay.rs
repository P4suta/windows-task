use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let corpus = PathBuf::from(arguments.next().unwrap_or_else(|| "fuzz/seeds".into()));
    let artifacts = PathBuf::from(arguments.next().unwrap_or_else(|| "target/verification/seed-replay".into()));
    fs::create_dir_all(&artifacts)?;
    let mut paths: Vec<_> = fs::read_dir(corpus)?.map(|entry| entry.map(|entry| entry.path())).collect::<Result<_, _>>()?;
    paths.sort();
    let seeds: Vec<_> = paths.iter().map(fs::read).collect::<Result<_, _>>()?;
    assert!(!seeds.is_empty(), "a seed corpus is required");
    let mut random = 20260905_u64;
    for iteration in 0..1000 {
        let mut input = seeds[iteration % seeds.len()].clone();
        if iteration >= seeds.len() && !input.is_empty() {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let offset = (random as usize) % input.len();
            input[offset] = (random >> 32) as u8;
            if iteration % 7 == 0 { input.truncate(offset); }
        }
        if std::panic::catch_unwind(|| windows_task_fuzz::exercise(&input)).is_err() {
            let path = artifacts.join(format!("seed-20260905-iteration-{iteration}.bin"));
            fs::write(&path, input)?;
            return Err(format!("input invariant failed; reproduce with {}", path.display()).into());
        }
    }
    fs::write(artifacts.join("result.txt"), "seed=20260905 samples=1000 passed; deterministic replay, not coverage-guided fuzzing\n")?;
    println!("seed=20260905 samples=1000 passed (deterministic replay)");
    Ok(())
}
