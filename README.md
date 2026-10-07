# Overview
This project contains a shell program written entirely in the Rust programming language. It includes the essential features that any shell needs, including pipelines, logical operators, redirection, and more.

# Aliases
Using aliases in this shell might be slightly unfamiliar. However, all that is required to set up an alias is to type the command 'alias [name]=[value]', with [name] being the alias and [value] being whatever command you wish to inject. To remove an existing alias, type 'unalias [name]'. To see a list of all existing aliases, simply type 'alias'.

# Notes
Most other features in the shell should be familiar and match the symbols and syntax found in most shell programs, such as Bash. If there is anything that is unclear, reviewing the code in src/main.rs should help identify the issue. 
