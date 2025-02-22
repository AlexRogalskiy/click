// Copyright 2021 Databricks, Inc.

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at

// http://www.apache.org/licenses/LICENSE-2.0

// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use ansi_term::Colour::Yellow;
use clap::{App, Arg};
use rustyline::completion::Pair as RustlinePair;

use crate::{
    command::command_def::{exec_match, start_clap, Cmd},
    completer,
    env::Env,
    error::ClickError,
    kobj::KObj,
    output::ClickWriter,
};

use std::array::IntoIter;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::process::Command;

/// a clap validator for boolean
fn valid_bool(s: String) -> Result<(), String> {
    s.parse::<bool>().map(|_| ()).map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn do_exec(
    env: &Env,
    pod: &KObj,
    kluster_name: &str,
    cmd: &[&str],
    it_arg: &str,
    cont_opt: &Option<&str>,
    term_opt: &Option<&str>,
    do_terminal: bool,
    writer: &mut ClickWriter,
) -> Result<(), ClickError> {
    let ns = pod.namespace.as_ref().unwrap();
    if do_terminal {
        let terminal = if let Some(t) = term_opt {
            t
        } else if let Some(ref t) = env.click_config.terminal {
            t
        } else {
            "xterm -e"
        };
        let mut targs: Vec<&str> = terminal.split_whitespace().collect();
        let mut kubectl_args = vec![
            "kubectl",
            "--namespace",
            ns,
            "--context",
            kluster_name,
            "exec",
            it_arg,
            pod.name(),
        ];
        targs.append(&mut kubectl_args);
        if let Some(cont) = cont_opt {
            targs.push("-c");
            targs.push(cont);
        }
        targs.push("--");
        targs.extend(cmd.iter());
        clickwriteln!(writer, "Starting on {} in terminal", pod.name());
        duct::cmd(targs[0], &targs[1..]).start()?;
        Ok(())
    } else {
        let mut command = Command::new("kubectl");
        command
            .arg("--namespace")
            .arg(ns)
            .arg("--context")
            .arg(kluster_name)
            .arg("exec")
            .arg(it_arg)
            .arg(pod.name());
        let command = if let Some(cont) = cont_opt {
            command.arg("-c").arg(cont).arg("--").args(cmd)
        } else {
            command.arg("--").args(cmd)
        };
        match command.status() {
            Ok(s) => {
                if s.success() {
                    Ok(())
                } else {
                    Err(ClickError::CommandError(
                        "kubectl exited abnormally".to_string(),
                    ))
                }
            }
            Err(e) => {
                if let io::ErrorKind::NotFound = e.kind() {
                    Err(ClickError::CommandError(
                        "Could not find kubectl binary. Is it in your PATH?".to_string(),
                    ))
                } else {
                    Err(ClickError::Io(e))
                }
            }
        }
    }
}

command!(
    Exec,
    "exec",
    "exec specified command on active pod",
    |clap: App<'static, 'static>| clap
        .arg(
            Arg::with_name("command")
                .help("The command to execute")
                .required(true)
                .multiple(true) // required for trailing_var_arg
                .index(1)
        )
        .arg(
            Arg::with_name("container")
                .short("c")
                .long("container")
                .help("Exec in the specified container")
                .takes_value(true)
        )
        .arg(
            Arg::with_name("terminal")
                .short("t")
                .long("terminal")
                .help(
                    "Run the command in a new terminal.  With --terminal ARG, ARG is used as the \
                     terminal command, otherwise the default is used ('set terminal <value>' to \
                     specify default). If a range of objects is selected, a new terminal is opened \
                     for each object."
                )
                .takes_value(true)
                .min_values(0)
        )
        .arg(
            Arg::with_name("tty")
                .short("T")
                .long("tty")
                .help("If stdin is a TTY. Contrary to kubectl, this defaults to TRUE")
                .validator(valid_bool)
                .takes_value(true)
                .min_values(0)
        )
        .arg(
            Arg::with_name("stdin")
                .short("i")
                .long("stdin")
                .help("Pass stdin to the container. Contrary to kubectl, this defaults to TRUE")
                .validator(valid_bool)
                .takes_value(true)
                .min_values(0)
        ),
    vec!["exec"],
    noop_complete!(),
    IntoIter::new([(
        "container".to_string(),
        completer::container_completer as fn(&str, &Env) -> Vec<RustlinePair>
    )])
    .collect(),
    |matches, env, writer| {
        let cmd: Vec<&str> = matches.values_of("command").unwrap().collect(); // safe as required
        if let Some(context) = env.context.as_ref() {
            let tty = if matches.is_present("tty") {
                if let Some(v) = matches.value_of("tty") {
                    // already validated
                    v.parse::<bool>().unwrap()
                } else {
                    true
                }
            } else {
                true
            };
            let stdin = if matches.is_present("stdin") {
                if let Some(v) = matches.value_of("stdin") {
                    // already validated
                    v.parse::<bool>().unwrap()
                } else {
                    true
                }
            } else {
                true
            };
            let it_arg = match (tty, stdin) {
                (true, true) => "-it",
                (true, false) => "-t",
                (false, true) => "-i",
                (false, false) => "",
            };
            env.apply_to_selection(
                writer,
                Some(&env.click_config.range_separator),
                |obj, writer| {
                    if obj.is_pod() {
                        do_exec(
                            env,
                            obj,
                            &context.name,
                            &cmd,
                            it_arg,
                            &matches.value_of("container"),
                            &matches.value_of("terminal"),
                            matches.is_present("terminal"),
                            writer,
                        )
                    } else {
                        Err(ClickError::CommandError(
                            "Exec only possible on pods".to_string(),
                        ))
                    }
                },
            )
        } else {
            Err(ClickError::CommandError(
                "Need an active context in order to exec.".to_string(),
            ))
        }
    },
    true // exec wants to gather up all it's training args into one big exec call
);
