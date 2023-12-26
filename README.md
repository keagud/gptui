# GPTui

**GPTui** is a fairly simple (TUI)[https://en.wikipedia.org/wiki/Text-based_user_interface] for interacting with the OpenAI Chat Completions API.

## Why does this exist?
I prefer the question-and-answer style of interaction with LLMS for programming assistance (as opposed to something like Copilot's autocomplete), and I wanted to make an interface designed with that in mind, that could also slot into my existing CLI/TUI based workflow. If you're a fellow vim/tmux/alacritty enjoyer, consider giving it a shot!

## Features:
- Token streaming for that cool ChatGPT vibe
- Optionally provide your API key at compile time, so you don't need to futz with environment variables later
- Configure custom prompts and other settings in a TOML config file
- Chat history is saved to a local Sqlite database for later
- Syntax highlighting in code blocks
- Copy code block content to your system clipboard
- Optionally compose messages in your favorite text editor (ctrl-e to open)
- All in 100% safe, blazingly fast rust! (*blazing-fastness may vary based on network conditions*)

## Authentication
You'll need an (OpenAI API key)[https://platform.openai.com/docs/api-reference/authentication] to get started. Anecdotally, GPTui is pretty cheap, even with heavy usage, since it's all text and doesn't use any of the multi-modal bells and whistles of the API. You can provide your API key in one of two ways:
- **As an environment variable:** The program will look for an OPENAI_API_KEY environment variable at runtime by default; you can change the name of this variable in `config.toml` (see below for more on that.)
- **Provided at compile time:**, If you build with the `comptime-key` feature enabled, the value of OPENAI_API_KEY will be read and compiled into the binary itself. This means you don't have to keep the key in the environment, but if you change your key you'll need to recompile. 

## Config
When first run, a commented `config.toml` file will be generated wherever config files belong on your platform (on Linux it's $XDG_CONFIG_HOME). 


## CLI
```
Start a new conversation thread
Usage: gpt new [OPTIONS]

Options:
  -p, --prompt <PROMPT>  Prompt to use
  -h, --help             Print help
```

```
Resume a previous conversation
Usage: gpt resume <INDEX>

Arguments:
  <INDEX>  

Options:
  -h, --help  Print help
```
```
Delete a conversation thread permanently
Usage: gpt delete <INDEX>

Arguments:
  <INDEX>  

Options:
  -h, --help  Print help

```
```
List all saved threads
Usage: gpt list

Options:
  -h, --help  Print help
```
```
Delete all conversation threads
Usage: gpt clear

Options:
  -h, --help  Print help
```

## Key Bindings
- Alt-Enter: submit message
- Ctrl-e: Open the message buffer in an external editor ($EDITOR if available, a safe default for the platform if not). Save and quit the editor window when you're done to return.
- Ctrl-w: Enter copy mode
    - In copy mode, enter the number of a code block and press Enter to copy its contents to the system clipboard.
- Esc: Exit copy mode
- Up/Down: Scroll the chat history 
- Ctrl-c: Exit the program


# Roadmap/Wishlist
[x] External editor support
[x] Syntax highlighting
[x] Copy code blocks to clipboard on:
    [x] Windows 
    [x] Linux (X11)
    [ ] Linux (Wayland) (partially working)
    [ ] MacOS (I can't test this without a Mac, but I think it has a good chance of working already)
[ ] Document key bindings in the UI itself
[ ] Replace CLI for selecting a thread with a TUI screen?

