use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::process::{Command, Stdio, Child};
use std::path::Path;
use std::env;

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

    format!("{} > ", path_str)
}

fn main() {
    // Using rustyline to handle user input
    let mut rl = DefaultEditor::new().unwrap();
    let _ = rl.load_history("history.txt");

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

                // Separating commands if using pipes
                let mut commands = trimmed.split(" | ").peekable();
                let mut previous_command: Option<Child> = None;

                while let Some(command) = commands.next() {
                    let mut parts = command.trim().split_whitespace();
                    let command = match parts.next() {
                        Some(cmd) => cmd,
                        None => continue,
                    };
                    let args = parts;

                    match command {
                        // Explicit command for changing directory
                        "cd" => {
                            let new_dir = args.peekable().peek().map_or("/", |x| *x);
                            let root = Path::new(new_dir);
                            if let Err(e) = env::set_current_dir(&root) {
                                eprintln!("{}", e);
                            }
                            previous_command = None;
                        }
                        // Command for printing current directory
                        "pwd" => {
                            if let Ok(path) = env::current_dir() {
                                println!("{}", path.display());
                            } else {
                                eprintln!("Failed to get current directory");
                            }
                            previous_command = None;
                        }
                        // Command for exiting shell
                        "exit" => {
                            let _ = rl.save_history("history.txt");
                            return;
                        }
                        // External commands & pipelines
                        command => {
                            let stdin = previous_command.map_or(
                                Stdio::inherit(),
                                |output: Child| Stdio::from(output.stdout.unwrap()),
                            );

                            let stdout = if commands.peek().is_some() {
                                Stdio::piped()
                            } else {
                                Stdio::inherit()
                            };

                            let output = Command::new(command)
                                .args(args)
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