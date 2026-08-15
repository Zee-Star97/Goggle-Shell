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
        if c == '$' {
            let mut var_name = String::new();
            
            // Extracting variable name
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
                // Matching to either session or environment variable
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
        // Detecting wildcard characters
        if arg.contains('*') || arg.contains('?') {
            if let Ok(paths) = glob(arg) {
                let mut matched = false;
                // Using wildcards to filter results
                for path in paths.filter_map(Result::ok) {
                    expanded_args.push(path.to_string_lossy().into_owned());
                    matched = true;
                }
                // Pass whole string if no matches
                if !matched {
                    expanded_args.push(arg.to_string());
                }
            } else {
                expanded_args.push(arg.to_string());
            }
        } else {
            expanded_args.push(arg.to_string());
        }
    }

    expanded_args
}


// Helper function for 'ls' command
fn execute_ls<'a>(args: impl Iterator<Item = &'a str>) {
    let mut show_hidden = false;
    let mut long_format = false;
    let mut target_dir = String::from(".");

    // Parsing arguments
    for arg in args {
        if arg.starts_with('-') {
            if arg.contains('a') { show_hidden = true; } // '-a' for hidden files
            if arg.contains('l') { long_format = true; } // '-l' for extra information
        } else {
            target_dir = arg.to_string();
        }
    }

    // Reading entries within target directory
    if let Ok(entries) = fs::read_dir(&target_dir) {
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name()); // Sorting alphabetically

        for entry in entries {
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            // Handling hidden files
            if !show_hidden && name_str.starts_with('.') {
                continue;
            }

            // Giving directories a unique colour
            let display_name = if entry.path().is_dir() {
                name_str.blue().bold().to_string()
            } else {
                name_str.to_string()
            };

            // Handling extra information 
            if long_format {
                if let Ok(metadata) = entry.metadata() {
                    let is_dir = metadata.is_dir();
                    let size = metadata.len();
                    
                    // Reading file/directory permissions
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

                    // Using chrono to display last modifid time
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
    } else {
        eprintln!("ls: cannot access '{}': No such file or directory", target_dir);
    }
}

fn main() {
    // Using rustyline to handle user input
    let mut rl = DefaultEditor::new().unwrap();
    let _ = rl.load_history("history.txt");

    // HashMap to keep track of session variables
    let mut session_vars: HashMap<String, String> = HashMap::new();

    loop {
        // Starting a new prompt line
        let prompt = get_prompt();
        let readline = rl.readline(&prompt);

        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }

                // Adding valid inputs to user history
                let _ = rl.add_history_entry(trimmed);

                // Expanding session variables
                let expanded_line = expand_variables(trimmed, &session_vars);

                // Separating commands if using pipes
                let mut commands = expanded_line.split(" | ").peekable();
                let mut previous_command: Option<Child> = None;

                while let Some(full_command) = commands.next() {
                    let mut command_str = full_command.trim();
                    let mut outfile = None;

                    // Parsing input for redirection operator '>'
                    if let Some(idx) = command_str.find('>') {
                        outfile = Some(command_str[idx + 1..].trim());
                        command_str = command_str[..idx].trim();
                    }

                    let mut parts = command_str.split_whitespace();
                    let command = match parts.next() {
                        Some(cmd) => cmd,
                        None => continue,
                    };

                    // Expanding glob characters
                    let expanded_args = expand_globs(parts);

                    match command {
                        // Explicit command for changing directory
                        "cd" => {
                            let default_home = dirs::home_dir(); // Home directory as default
                            let default_str = default_home
                                .as_ref()
                                .and_then(|p| p.to_str())
                                .unwrap_or("/");

                            let new_dir = expanded_args.first().map(|s| s.as_str()).unwrap_or(default_str);
                            let root = Path::new(new_dir);
                            if let Err(e) = env::set_current_dir(&root) {
                                eprintln!("{}", e);
                            }
                            previous_command = None;
                        }
                        // Command for printing current directory
                        "pwd" => {
                            if let Ok(path) = env::current_dir() {
                                println!("{}", path.display().to_string().cyan());
                            } else {
                                eprintln!("Failed to get current directory");
                            }
                            previous_command = None;
                        }
                        // Command for listing files and directories
                        "ls" => {
                            execute_ls(expanded_args.iter().map(|s| s.as_str()));
                            previous_command = None;
                        }
                        // Command for exiting shell
                        "exit" => {
                            let _ = rl.save_history("history.txt");
                            return;
                        }
                        // Variable assignment catch
                        cmd if cmd.contains('=') => {
                            let mut split = cmd.splitn(2, '=');
                            if let (Some(key), Some(value)) = (split.next(), split.next()) {
                                session_vars.insert(key.to_string(), value.to_string());
                            }
                            previous_command = None;
                        }
                        // External commands & pipelines
                        command => {
                            let stdin = previous_command.map_or(
                                Stdio::inherit(),
                                |output: Child| Stdio::from(output.stdout.unwrap()),
                            );

                            // Determining stdout file destination
                            let stdout = if let Some(file_name) = outfile {
                                match File::create(file_name) {
                                    Ok(file) => Stdio::from(file),

                                    // Continue without saving on error
                                    Err(e) => {     
                                        eprintln!("Failed to open file {}: {}", file_name, e);
                                        previous_command = None;
                                        continue;
                                    }
                                }
                            } else if commands.peek().is_some() {
                                Stdio::piped()
                            } else {
                                Stdio::inherit()
                            };

                            let output = Command::new(command)
                                .args(&expanded_args)
                                .stdin(stdin)
                                .stdout(stdout)
                                .spawn();

                            match output {
                                Ok(output) => { previous_command = Some(output); }
                                Err(e) => {
                                    previous_command = None;
                                    eprintln!("{}", e);
                                }
                            }
                        }
                    }
                }

                // Waiting until commands are finished before looping
                if let Some(mut final_command) = previous_command {
                    let _ = final_command.wait();
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Handling interrupt signals without closing shell
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