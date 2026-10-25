//! CLI entrypoint: global-flag parsing and command dispatch.
//! Each command delegates to a `crate::commands::*` implementation;
//! this module owns no command business logic.

pub mod args;

use std::process::ExitCode;
use crate::protocol::tree::Result;
use crate::commands::{editing, model, place, render, runtime, sessions, studio, terrain, transform};

pub fn run_main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::from(0),
        Err(err) => {
            eprintln!("error: {}", err.0);
            if err.0.starts_with("usage:") { ExitCode::from(2) } else { ExitCode::from(1) }
        }
    }
}

use args::{usage_err, print_help, Global, VERSION};

pub fn run(args: &[String]) -> Result<()> {
    let mut global = Global { json: false, engine: None };
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => global.json = true,
            "--engine" => {
                i += 1;
                global.engine = Some(
                    args.get(i)
                        .ok_or_else(|| usage_err("--engine requires a path"))?
                        .clone(),
                );
            }
            "-h" | "--help" if rest.is_empty() => {
                print_help();
                return Ok(());
            }
            "-V" | "--version" if rest.is_empty() => {
                println!("bloxcli {VERSION}");
                return Ok(());
            }
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }
    let Some(command) = rest.first().cloned() else {
        print_help();
        return Err(usage_err("no command given"));
    };
    let cmd_args = &rest[1..];
    match command.as_str() {
        "studio" => studio::cmd_studio(&global, cmd_args),
        "studio-daemon" => studio::cmd_studio_daemon(&global, cmd_args),
        "mcp" => sessions::cmd_mcp(&global, cmd_args),
        "run" => runtime::cmd_run(&global, cmd_args),
        "play" => sessions::cmd_play(&global, cmd_args),
        "play-daemon" => sessions::cmd_play_daemon(&global, cmd_args),
        "net" => sessions::cmd_net(&global, cmd_args),
        "debug" => sessions::cmd_debug(&global, cmd_args),
        "debug-daemon" => sessions::cmd_debug_daemon(&global, cmd_args),
        "script" => runtime::cmd_script(&global, cmd_args),
        "net-daemon" => sessions::cmd_net_daemon(&global, cmd_args),
        "terrain" => terrain::cmd_terrain(&global, cmd_args),
        "place" => place::cmd_place(&global, cmd_args),
        "diff" => place::cmd_diff(&global, cmd_args),
        "imagediff" => place::cmd_imagediff(&global, cmd_args),
        "parity" => place::cmd_parity(&global, cmd_args),
        "model" => model::cmd_model(&global, cmd_args),
        "template" => place::cmd_template(&global, cmd_args),
        "new" => place::cmd_new(&global, cmd_args),
        "inspect" | "tree" => editing::cmd_tree(&global, cmd_args),
        "add" => editing::cmd_add(&global, cmd_args),
        "set" => editing::cmd_set(&global, cmd_args),
        "save" => editing::cmd_save(&global, cmd_args),
        "export" => editing::cmd_export(&global, cmd_args),
        "validate" => editing::cmd_validate(&global, cmd_args),
        "get" => editing::cmd_get(&global, cmd_args),
        "find" => editing::cmd_find(&global, cmd_args),
        "clone" => editing::cmd_clone(&global, cmd_args),
        "delete" => editing::cmd_delete(&global, cmd_args),
        "reparent" => editing::cmd_reparent(&global, cmd_args),
        "group" => editing::cmd_group(&global, cmd_args),
        "ungroup" => editing::cmd_ungroup(&global, cmd_args),
        "exec" => runtime::cmd_exec(&global, cmd_args),
        "move" => transform::cmd_move(&global, cmd_args),
        "rotate" => transform::cmd_rotate(&global, cmd_args),
        "scale" => transform::cmd_scale(&global, cmd_args),
        "pivot" => transform::cmd_pivot(&global, cmd_args),
        "undo" => editing::cmd_undo(&global, cmd_args, false),
        "render" => render::cmd_render(&global, cmd_args),
        "camera" => render::cmd_camera(&global, cmd_args),
        "ui" => render::cmd_ui(&global, cmd_args),
        "redo" => editing::cmd_undo(&global, cmd_args, true),
        "help" => {
            print_help();
            Ok(())
        }
        other => Err(usage_err(&format!("unknown command '{other}'"))),
    }
}

