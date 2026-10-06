use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::collections::HashMap;
use std::process::{Command, Stdio, Child};
use std::path::Path;
use std::env;
use std::fs::{self, File};
use colored::Colorize;
use glob::glob;

// Function to generate the current directory prompt that prefixes user input 
fn get_prompt() -> String {
    let current_dir = env::current_dir().unwrap_or_default();
    
    let path_str = if let Some(home_dir) = dirs::home_dir() {
        if let Ok(strip_path) = current_dir.strip_prefix(&home_dir) {
            format!("~{}", Path::new("/").join(strip_path).display())
        } else {
            current_dir.display().to_string()
        }
    } else {
        current_dir.display().to_string()
    };

    format!("{} {} ", path_str.cyan().bold(), ">".green().bold())
}

// Helper function to expand session variables
fn expand_variables(input: &str, session_vars: &HashMap<String, String>) -> String {
    let mut result = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next_c) = chars.peek() {
                if next_c == '$' {
                    result.push('$');
                    chars.next();
                    continue;
                }
            }
            result.push('\\');
        } else if c == '$' {
            let mut var_name = String::new();
            
            while let Some(&next_c) = chars.peek() {
                if next_c.is_alphanumeric() || next_c == '_' {
                    var_name.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            
            if var_name.is_empty() {
                result.push('$');
            } else {
                if let Some(val) = session_vars.get(&var_name) {
                    result.push_str(val);
                } else if let Ok(val) = env::var(&var_name) {
                    result.push_str(&val);
                }
            }
        } else {
            result.push(c);
        }
    }
    
    result
}

// Helper function to expand wildcard/glob characters
fn expand_globs<'a>(args: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut expanded_args = Vec::new();

    for arg in args {
        if arg.contains('*') || arg.contains('?') {
            if let Ok(paths) = glob(arg) {
                let mut matched = false;
                for path in paths.filter_map(Result::ok) {
                    expanded_args.push(path.to_string_lossy().into_owned());
                    matched = true;
                }
                if !matched {
                    expanded_args.push(arg.replace("\\*", "*").replace("\\?", "?"));
                }
            } else {
                expanded_args.push(arg.replace("\\*", "*").replace("\\?", "?"));
            }
        } else {
            expanded_args.push(arg.to_string());
        }
    }

    expanded_args
}

#[derive(Debug)]
struct ParsedCommand {
    args: Vec<String>,
    outfile: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum LogicalOp {
    And,    // &&
    Or,     // ||
    Always, // ;
    None,
}

// Helper function to parse logical operators
fn parse_logic_chains(input: &str) -> Vec<(String, LogicalOp)> {
    let mut chains = Vec::new();
    let mut current_cmd = String::new();
    let mut chars = input.chars().peekable();
    let mut current_op = LogicalOp::None;

    while let Some(c) = chars.next() {
        if c == '\\' {
            current_cmd.push(c);
            if let Some(next_c) = chars.next() {
                current_cmd.push(next_c);
            }
        } else if c == '&' && chars.peek() == Some(&'&') {
            chars.next();
            chains.push((current_cmd.clone(), current_op));
            current_cmd.clear();
            current_op = LogicalOp::And;
        } else if c == '|' && chars.peek() == Some(&'|') {
            chars.next();
            chains.push((current_cmd.clone(), current_op));
            current_cmd.clear();
            current_op = LogicalOp::Or;
        } else if c == ';' {
            chains.push((current_cmd.clone(), current_op));
            current_cmd.clear();
            current_op = LogicalOp::Always;
        } else {
            current_cmd.push(c);
        }
    }
    
    if !current_cmd.trim().is_empty() {
        chains.push((current_cmd, current_op));
    }
    
    chains
}

// Helper function to serialise and parse pipelines
fn parse_pipeline(input: &str) -> Vec<ParsedCommand> {
    let mut pipeline = Vec::new();
    let mut current_args = Vec::new();
    let mut current_arg = String::new();
    let mut current_outfile = None;
    let mut is_redirect = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' && !in_single_quote {
            if let Some(next_c) = chars.next() {
                if next_c == '*' || next_c == '?' {
                    current_arg.push('\\');
                    current_arg.push(next_c);
                } else {
                    current_arg.push(next_c);
                }
            }
        } else if c == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        } else if c == '|' && !in_single_quote && !in_double_quote {
            if !current_arg.is_empty() {
                if is_redirect {
                    current_outfile = Some(current_arg.clone());
                } else {
                    current_args.push(current_arg.clone());
                }
                current_arg.clear();
            }

            pipeline.push(ParsedCommand {
                args: current_args.clone(),
                outfile: current_outfile.clone(),
            });
            current_args.clear();
            current_outfile = None;
            is_redirect = false;
        } else if c == '>' && !in_single_quote && !in_double_quote {
            if !current_arg.is_empty() {
                current_args.push(current_arg.clone());
                current_arg.clear();
            }
            is_redirect = true;
        } else if c.is_whitespace() && !in_single_quote && !in_double_quote {
            if !current_arg.is_empty() {
                if is_redirect {
                    current_outfile = Some(current_arg.clone());
                } else {
                    current_args.push(current_arg.clone());
                }
                current_arg.clear();
            }
        } else {
            current_arg.push(c);
        }
    }

    if !current_arg.is_empty() {
        if is_redirect {
            current_outfile = Some(current_arg);
        } else {
            current_args.push(current_arg);
        }
    }

    if !current_args.is_empty() || current_outfile.is_some() {
        pipeline.push(ParsedCommand {
            args: current_args,
            outfile: current_outfile,
        });
    }

    pipeline
}

// Helper function for 'ls' command
fn execute_ls<'a>(args: impl Iterator<Item = &'a str>) -> bool {
    let mut show_hidden = false;
    let mut long_format = false;
    let mut target_dir = String::from(".");

    for arg in args {
        if arg.starts_with('-') {
            if arg.contains('a') { show_hidden = true; } // Show hidden files
            if arg.contains('l') { long_format = true; } // Show extra information
        } else {
            target_dir = arg.to_string();
        }
    }

    if let Ok(entries) = fs::read_dir(&target_dir) {
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            if !show_hidden && name_str.starts_with('.') {
                continue;
            }

            let display_name = if entry.path().is_dir() {
                name_str.blue().bold().to_string()
            } else {
                name_str.to_string()
            };

            if long_format {
                if let Ok(metadata) = entry.metadata() {
                    let is_dir = metadata.is_dir();
                    let size = metadata.len();
                    
                    let perms = {
                        use std::os::unix::fs::PermissionsExt;
                        let mode = metadata.permissions().mode();
                        format!(
                            "{}{}{}{}{}{}{}{}{}{}",
                            if is_dir { 'd' } else { '-' },
                            if mode & 0o400 != 0 { 'r' } else { '-' },
                            if mode & 0o200 != 0 { 'w' } else { '-' },
                            if mode & 0o100 != 0 { 'x' } else { '-' },
                            if mode & 0o040 != 0 { 'r' } else { '-' },
                            if mode & 0o020 != 0 { 'w' } else { '-' },
                            if mode & 0o010 != 0 { 'x' } else { '-' },
                            if mode & 0o004 != 0 { 'r' } else { '-' },
                            if mode & 0o002 != 0 { 'w' } else { '-' },
                            if mode & 0o001 != 0 { 'x' } else { '-' }
                        )
                    };

                    let modified = metadata.modified().unwrap_or_else(|_| std::time::SystemTime::now());
                    let datetime: chrono::DateTime<chrono::Local> = modified.into();
                    let time_str = datetime.format("%b %e %H:%M").to_string();

                    println!("{} {:>8} {} {}", perms, size, time_str, display_name);
                }
            } else {
                print!("{}  ", display_name);
            }
        }
        
        if !long_format {
            println!();
        }
        true
    } else {
        eprintln!("ls: cannot access '{}': No such file or directory", target_dir);
        false
    }
}

fn main() {
    let mut rl = DefaultEditor::new().unwrap();
    let _ = rl.load_history("history.txt");

    // Hashmaps for session variables and aliases
    let mut session_vars: HashMap<String, String> = HashMap::new();
    let mut aliases: HashMap<String, String> = HashMap::new();

    loop {
        // Using rustyline to retrieve input
        let prompt = get_prompt();
        let readline = rl.readline(&prompt);

        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }

                let _ = rl.add_history_entry(trimmed);
                let expanded_line = expand_variables(trimmed, &session_vars);
                
                // Parsing the expanded line into a series of logical chains
                let chains = parse_logic_chains(&expanded_line);
                
                let mut last_success = true;
                let mut should_exit = false;

                for (cmd_str, op) in chains {
                    let cmd_str = cmd_str.trim();
                    if cmd_str.is_empty() { continue; }

                    // Continue based on previous command status
                    match op {
                        LogicalOp::And if !last_success => continue, // Executes on success
                        LogicalOp::Or if last_success => continue,   // Executes only on failure
                        LogicalOp::Always => {},                     // Executes regardless of previous result
                        LogicalOp::None => {},
                        _ => {}
                    }

                    // Parsing and separating command components
                    let pipeline = parse_pipeline(cmd_str);
                    if pipeline.is_empty() { continue; }

                    let mut previous_command: Option<Child> = None;
                    let mut commands = pipeline.into_iter().peekable();
                    let mut current_success = true;

                    while let Some(parsed_cmd) = commands.next() {
                        let mut raw_args = parsed_cmd.args;

                        // Expanding aliases
                        if let Some(cmd_name) = raw_args.first() {
                            if let Some(alias_val) = aliases.get(cmd_name) {
                                let mut expanded: Vec<String> = alias_val
                                    .split_whitespace()
                                    .map(String::from)
                                    .collect();
                                
                                // Appending arguments
                                expanded.extend(raw_args.into_iter().skip(1));
                                raw_args = expanded;
                            }
                        }

                        let mut args_iter = raw_args.into_iter();
                        let command = match args_iter.next() {
                            Some(cmd) => cmd,
                            None => continue,
                        };

                        // Expanding glob characters
                        let args_vec: Vec<String> = args_iter.collect();
                        let expanded_args = expand_globs(args_vec.iter().map(|s| s.as_str()));
                        let outfile = parsed_cmd.outfile;

                        // Matching commands to in-built list 
                        match command.as_str() {
                            "alias" => {
                                // Setting up command aliases
                                if expanded_args.is_empty() {
                                    // Typing simply 'alias' prints list of aliases 
                                    for (name, val) in &aliases {
                                        println!("alias {}='{}'", name, val);
                                    }
                                    current_success = true;
                                } else {
                                    // Assigning a new alias
                                    for arg in &expanded_args {
                                        if let Some((name, val)) = arg.split_once('=') {
                                            let clean_val = val.trim_matches(|c| c == '\'' || c == '"').to_string();
                                            aliases.insert(name.to_string(), clean_val);
                                            current_success = true;
                                        } else {
                                            // Print usage information
                                            eprintln!("alias: invalid format. Use name=\"value\"");
                                            current_success = false;
                                            break;
                                        }
                                    }
                                }
                                previous_command = None;
                            }
                            "unalias" => {
                                // Remove an alias entry
                                if expanded_args.is_empty() {
                                    eprintln!("unalias: incomplete command. Try: unalias [name]");
                                    current_success = false;
                                } else {
                                    for arg in &expanded_args {
                                        aliases.remove(arg);
                                    }
                                    current_success = true;
                                }
                                previous_command = None;
                            }
                            "cd" => {
                                // Changing directory
                                let default_home = dirs::home_dir();
                                let default_str = default_home.as_ref().and_then(|p| p.to_str()).unwrap_or("/");
                                let new_dir = expanded_args.first().map(|s| s.as_str()).unwrap_or(default_str);
                                
                                if let Err(e) = env::set_current_dir(Path::new(new_dir)) {
                                    eprintln!("{}", e);
                                    current_success = false;
                                } else {
                                    current_success = true;
                                }
                                previous_command = None;
                            }
                            "pwd" => {
                                // Print working directory
                                if let Ok(path) = env::current_dir() {
                                    println!("{}", path.display().to_string().cyan());
                                    current_success = true;
                                } else {
                                    eprintln!("Failed to get current directory");
                                    current_success = false;
                                }
                                previous_command = None;
                            }
                            "ls" => {
                                // List target directory contents
                                current_success = execute_ls(expanded_args.iter().map(|s| s.as_str()));
                                previous_command = None;
                            }
                            "exit" => {
                                // Exiting shell program
                                let _ = rl.save_history("history.txt");
                                should_exit = true;
                                break;
                            }
                            cmd if cmd.contains('=') => {
                                // Assigning a session variable
                                let mut split = cmd.splitn(2, '=');
                                if let (Some(key), Some(value)) = (split.next(), split.next()) {
                                    session_vars.insert(key.to_string(), value.to_string());
                                    current_success = true;
                                } else {
                                    current_success = false;
                                }
                                previous_command = None;
                            }
                            command => {
                                // Delegating unknown command to OS
                                let stdin = previous_command.map_or(
                                    Stdio::inherit(),
                                    |output: Child| Stdio::from(output.stdout.unwrap()),
                                );

                                let stdout = if let Some(ref file_name) = outfile {
                                    match File::create(file_name) {
                                        Ok(file) => Stdio::from(file),
                                        Err(e) => {
                                            eprintln!("{}: {}", command, e);
                                            current_success = false;
                                            previous_command = None;
                                            break;
                                        }
                                    }
                                } else if commands.peek().is_some() {
                                    Stdio::piped()
                                } else {
                                    Stdio::inherit()
                                };

                                // Generating command
                                let output = Command::new(command)
                                    .args(&expanded_args)
                                    .stdin(stdin)
                                    .stdout(stdout)
                                    .spawn();

                                match output {
                                    Ok(output) => { previous_command = Some(output); }
                                    Err(e) => {
                                        eprintln!("{}: {}", command, e);
                                        current_success = false;
                                        previous_command = None;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    // Checking command exit status
                    if let Some(mut final_command) = previous_command {
                        match final_command.wait() {
                            Ok(status) => current_success = status.success(),
                            Err(_) => current_success = false,
                        }
                    }
                    
                    last_success = current_success;
                    if should_exit { break; }
                }

                if should_exit { break; }

            }
            Err(ReadlineError::Interrupted) => {
                // Handling interrupt signal without closing shell
                println!("^C");
            }
            Err(ReadlineError::Eof) => {
                // Gracefully handling EoF signal
                println!("Exit");
                break;
            }
            Err(err) => {
                // General error catching
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    // Saving input history to a file for persistent storage
    let _ = rl.save_history("history.txt");
}