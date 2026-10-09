# AI agents (MCP)

NZAP Engine is also a [Model Context Protocol](https://modelcontextprotocol.io)
server. Claude Code, Claude Desktop, Cursor and other MCP clients can start
Colab runtimes on your account, run code there, move files in and out, and run
the NZAP apps, so heavy work such as speech to text, ffmpeg conversions or model
inference needs nothing installed on your machine.

The server is the installed app started with the `mcp` argument. The agent
launches it and talks to it over stdin/stdout. No port is opened, and the app
window does not need to be open.

## Set up

1. Install NZAP Engine and click **Connect Google** once in the app. The agent
   server uses the same connection.
2. Open **Settings → AI agents (MCP)**. It shows the exact command for this
   installation, because the path differs by OS and installer.
   - **Claude Code**: copy the one-liner, which looks like

     ```bash
     claude mcp add --scope user nzap -- '/path/to/nzap-engine' mcp
     ```

     `--scope user` makes it available in every project.

   - **Claude Desktop, Cursor and others**: copy the `mcpServers` JSON into the
     client's MCP configuration (`claude_desktop_config.json`, `.cursor/mcp.json`, …).

3. Ask the agent for something, for example _"Use NZAP to extract the audio
   from talk.mp4 as MP3"_ or _"Transcribe interview.wav on a T4 with NZAP"_.

On Linux, the AppImage is launched as the AppImage file itself. Settings shows
that path rather than the temporary mount the app runs from.

## Tools

| Tool             | What it does                                                                                          |
| ---------------- | ----------------------------------------------------------------------------------------------------- |
| `status`         | Google connection, runtimes held and limits, hardware names, shared folders                           |
| `list_runtimes`  | This agent's runtimes, plus other VMs on the account it could attach                                  |
| `start_runtime`  | Allocate a VM (`cpu`, `t4`, `l4`, `g4`, `a100`, `h100`, `v5e1`, `v6e1`; optional High-RAM)            |
| `attach_runtime` | Use a VM the agent did not start (for example one the app manages); it is never released by the agent |
| `stop_runtime`   | Release a runtime (or detach from an attached one)                                                    |
| `run_code`       | Run Python in the runtime's IPython kernel; returns output, results, errors and images                |
| `list_files`     | List a folder on the runtime                                                                          |
| `upload_file`    | Local file → runtime                                                                                  |
| `download_file`  | Runtime → local file                                                                                  |
| `run_job`        | One call: fresh VM, upload `inputs`, run a script, download `artifacts`, release the VM               |
| `list_apps`      | The NZAP apps (text to speech, sentiment, …) with parameters, recommended hardware and timings        |
| `run_app`        | Run an app; media results are saved locally, text results returned; the model stays warm for reruns   |

Long calls (`run_job`, `run_app`) report progress to clients that ask for it, and
a client can cancel any call.

## How it behaves

- **One Google connection.** The server reads the connection the app stored in
  the OS keychain. If you connect (or reconnect) in the app while an agent is
  running, the agent picks it up on its next call. Tokens never reach the agent.
- **Its own runtimes.** Each server process keeps its own runtime list, separate
  from the app's. The app lists agent runtimes under **External runtimes**
  (importing one there lets the app use it too, but the agent still releases it
  when it disconnects).
- **Released on disconnect.** When the agent disconnects (or the server is
  stopped), every runtime it started is released, unless the agent asked for
  `keep_after_disconnect` or the server runs with `--keep-runtimes`. Attached
  runtimes are only detached. If the server is killed outright, its runtimes
  stop being kept alive and Colab reclaims them after its usual idle timeout.
- **A limit.** One agent holds at most 2 runtimes at once (`--max-runtimes`),
  counting a running job's VM. Each one uses compute units until it stops.
- **Local files stay inside shared folders.** The agent can read and write only
  inside the folder the server started in (the project, for Claude Code) and any
  `--allow-dir`. `..` and symbolic links cannot reach outside. Results go to
  `nzap-output/` in that folder unless the agent names another folder inside it.
- **File size.** Files up to 500 MB move in either direction (`--max-file-mb`).
  Uploads above 8 MB are sent in chunks.
- **Code runs as in the console.** `!ffmpeg …` and `%pip install …` work, and
  state persists between `run_code` calls. `input()` receives an empty line. A
  cell that asks for Drive or Google Cloud consent is interrupted, with the link
  to give the user. Mount Drive from the app instead.

## Options

```
nzap-engine mcp [--allow-dir <folder>]… [--output-dir <folder>]
                [--max-runtimes <n>] [--max-file-mb <n>] [--keep-runtimes]
```

| Option                  | Default         | Meaning                                                  |
| ----------------------- | --------------- | -------------------------------------------------------- |
| `--allow-dir <folder>`  | —               | Also share this folder (repeatable)                      |
| `--output-dir <folder>` | `./nzap-output` | Default place for results (shared too)                   |
| `--max-runtimes <n>`    | 2               | Runtimes this agent may hold at once                     |
| `--max-file-mb <n>`     | 500             | Largest file uploaded or downloaded                      |
| `--keep-runtimes`       | off             | Do not release this agent's runtimes when it disconnects |

With Claude Code, put the options after `mcp`:
`claude mcp add --scope user nzap -- '/path/to/nzap-engine' mcp --allow-dir ~/Movies`.

The server logs to stderr, which MCP clients keep in their own logs. Set
`NZAP_LOG=debug` for more detail.

## Examples

These are the calls an agent typically makes. You only describe the task.

**Extract the audio from a video** (CPU is enough; ffmpeg is preinstalled on
Colab):

```json
{
  "name": "run_job",
  "arguments": {
    "inputs": [{ "local_path": "talk.mp4" }],
    "script": "import subprocess\nsubprocess.run(['ffmpeg', '-i', 'talk.mp4', '-vn', '-q:a', '2', 'talk.mp3'], check=True)",
    "artifacts": ["talk.mp3"]
  }
}
```

**Speech to text** on a T4 with faster-whisper:

```json
{
  "name": "run_job",
  "arguments": {
    "hardware": "t4",
    "inputs": [{ "local_path": "interview.wav" }],
    "script": "import subprocess, sys\nsubprocess.run([sys.executable, '-m', 'pip', 'install', '-q', 'faster-whisper'], check=True)\nfrom faster_whisper import WhisperModel\nmodel = WhisperModel('small', device='cuda', compute_type='float16')\nsegments, _ = model.transcribe('interview.wav')\nwith open('transcript.txt', 'w') as out:\n    for s in segments:\n        out.write(f'[{s.start:.1f}s] {s.text.strip()}\\n')",
    "artifacts": ["transcript.txt"],
    "timeout_seconds": 1800
  }
}
```

**Text to speech** with an NZAP app (the model stays loaded for the next call):

```json
{
  "name": "run_app",
  "arguments": { "app": "kokoro-tts", "params": { "text": "Hello!", "voice": "af_heart" } }
}
```

For several steps on the same data, `start_runtime` once, then `upload_file`,
`run_code` (as often as needed) and `download_file`, then `stop_runtime`.

## Security

- The server is a child process of the agent's client on stdin/stdout. It opens
  no port, so other processes and web pages cannot reach it. Anything that can
  launch it already runs as you.
- An agent with these tools can spend your compute units, run code on your
  Colab VMs, and read and write files in the shared folders. MCP clients ask
  before each tool call unless you allow it permanently. Claude Code's
  permission prompts work this way.
- The agent never sees your Google tokens, only results.

See [SECURITY.md](../SECURITY.md) for the full threat model.
