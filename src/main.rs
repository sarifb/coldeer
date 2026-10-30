use coldeer::{workflow as w, Result};
use std::{
    env,
    fs::File,
    io::{self, BufRead},
    path::Path,
    process::ExitCode,
};

const HELP: &str ="Coldeer 0.1.0\n\n  coldeer import SPEC.json OUTPUT_PATH_FILE\n  coldeer container PLAN.json plan|build|save|push [ARGS...]\n  coldeer base-image SPEC.json validate|update|lock [--out FILE]\n  coldeer scan-references NAR CANDIDATES_FILE\n  coldeer --version\n\nSee README.md and docs/interfaces.md for schema version 1.";

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [command, nar, refs] if command == "scan-references" => {
            let candidates = io::BufReader::new(File::open(refs)?)
                .lines()
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for path in coldeer::scan_references(File::open(nar)?, &candidates)? {
                println!("{path}");
            }
        }
        [command, spec, out] if command == "import" => w::import(Path::new(spec), Path::new(out))?,
        [command, spec, action, rest @ ..] if command == "container" => {
            w::container(Path::new(spec), action, rest)?
        }
        [command, spec, action] if command == "base-image" => {
            w::base_image(Path::new(spec), action, None)?
        }
        [command, spec, action, flag, out] if command == "base-image" && flag == "--out" => {
            w::base_image(Path::new(spec), action, Some(Path::new(out)))?
        }
        [flag] if flag == "--version" => println!("coldeer 0.1.0"),
        [] => println!("{HELP}"),
        [flag] if flag == "--help" || flag == "-h" => println!("{HELP}"),
        _ => return Err("invalid command; run coldeer --help".into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("coldeer: {e}");
            ExitCode::FAILURE
        }
    }
}
