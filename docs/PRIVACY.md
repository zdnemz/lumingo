# Privacy

Lumingo runs on your own computer. This page says what it sends out, what it
keeps, and where each of those lives. Every statement here comes from the
program's own code.

Two things are true before any detail, and the rest of this page is the
explanation:

- Your voice stays on your device.
- Text is sent to the AI provider you choose.

## What leaves your computer

Text is sent to the AI provider you choose.

You choose the service and you pay it yourself. Lumingo ships without a key and
cannot talk to a provider without one.

The prompts carry:

- what you type,
- what is written down from your speech,
- your writing drafts,
- the lesson context inside the prompts.

Your key leaves as well, and it goes to the same place: the base URL of the
profile you are using, as the sign-in header that service expects. Nothing
else about your account is sent.

What does not leave:

- **Raw audio.** It is not sent to the provider.
- **Your voice.** It never leaves the device.

Raw audio is not kept either, unless you switch on recordings in Settings. When
you do, the recording files are written into the `audio/` folder in the data
folder and stay there.

## What stays on your computer

- **The audio path.** Your microphone is read by the program on this computer.
  Audio is handled here and never reaches the browser tab or the network. The
  speech engines that would turn it into text also run here.
- **Your work.** Your sessions, turns, transcripts, drafts, attempts and scores
  are written to the database on this disk.
- **Your estimates.** Level estimates are computed on this computer, from the
  attempts stored here. No model output sets or states a level.
- **Your name, your device information and your usage statistics.** They stay
  here. There is no analytics and no update check, so nothing is sent to count
  your use of the program.

## Where your data is stored

Everything the program keeps lives in one folder: `%LOCALAPPDATA%\Lumingo` on
Windows, for example `C:\Users\YourName\AppData\Local\Lumingo`. It holds:

- `lumingo.sqlite` — the database file, with your history, your attempts and
  your settings.
- `providers.toml` — the provider profiles you add inside the program. It
  exists only if you add one.
- `models/` — the speech models you install, one folder per model.
- `audio/` — the recordings, only if you switch on recordings in Settings.

The session cookie is named `tutor_session`. It holds a random value the
program makes at start-up, it is not written to disk, and it is gone when the
program stops. The program listens on `http://127.0.0.1:8765` by default, on
the loopback address only. If that port is taken, the program takes the next
free one and prints the address it is using.

The first-run steps, including what to put in the `.env` file, are in
[RELEASE_README.md](RELEASE_README.md).

## Where your key is stored

Your key is saved in plain text in `providers.toml` in that data folder, or it
is read from the `.env` file you fill in next to the program. Either way it is
a plain-text file on your own disk, and any program running as you can read
it. That is the trade the program makes on purpose: the file is yours, and
nothing in it is hidden.

The key goes only to the base URL of the profile you use. The full key is
never printed by the program, and it is never written to the export. The
interface shows the last four characters of the key, so you can tell two keys
apart, and nothing more.

A provider profile needs four settings: `TUTOR_LLM_PROTOCOL`,
`TUTOR_LLM_BASE_URL`, `TUTOR_LLM_MODEL` and `TUTOR_LLM_API_KEY`. The template
for them is `.env.example`, which ships with empty values.

## Outbound hosts

Every outbound request goes through one allowlist, and the allowlist holds:

- the `host:port` of the provider profile you are using, and nothing else, and
- the model download hosts, `github.com` and `huggingface.co`, and only while
  the setup guard is held — that is, only while the download is running. Drop
  the guard and those hosts are refused again.

A request to any other host is refused before a socket is opened, and so is a
redirect to a host that is not on the list. The client does not use the
system's proxy settings, so there is no second destination outside the
allowlist.

The scheme is checked as well: HTTPS is required. Plain HTTP is allowed only
for the loopback address, so that a provider running on this computer can be
used.

The browser tab contacts only the local server. It talks to the program on
`http://127.0.0.1:8765` and to nothing else.

The payload inspector shows the requests and responses that were actually
sent. It keeps the last `50` of them. A request or response body is cut at
`16 KiB`, and the entry says that it was cut. The inspector stores no
headers, and every body is scrubbed of the key before it is stored. It lives
in the program's memory, it is shown only on the loopback address, and it is
not part of the export.

## What is not yet proven

- **No packet capture of a full session has been done.** That is the owner's
  step: one capture with a provider configured, one during setup, and the
  browser's network log for a full session. Until it is done, the host list
  above comes from reading the code, not from watching traffic go out.
- **The payload inspector's populated state is proven only in tests.** The
  tests show that the ring keeps the newest entries, drops the oldest and
  counts what it dropped, cuts a long body at the character boundary, and
  removes the key from both the request body and the response body. Nothing
  yet shows the inspector filled by a real session.
- **The Windows release has not been run by an agent.** The release steps, and
  the list of what has and has not been executed, are in
  [OWNER_RELEASE.md](OWNER_RELEASE.md).
