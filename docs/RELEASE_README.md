# Lumingo

Lumingo is an English practice program that runs on your own computer. This
README is for a learner who has never set up an AI service before.

## What this is

- It is a program, not a page you open online. You start it on your own
  computer and it serves its pages from there.
- It listens only on your own computer, on the address
  `http://127.0.0.1:8765`. Nothing else on your network can reach it.
- The tutor's answers come from an AI service that you choose and pay for
  yourself. Lumingo does not include a key, and it cannot work without one.
- What leaves your computer: the text you type, and your key. Both go to the
  service you chose. Nothing else does.

## What you need

64-bit Windows, 8 GB RAM, an x86-64 CPU with AVX2 and at least 4 threads, about 2 GB of free disk, a current browser, a microphone for the speaking parts, and an internet connection for the tutor.

This build has no microphone support and no speech engines, so the speaking
parts do not work here. The parts that use text do.

You also need:

- An account with an AI service, and a key from it. The key is a long string
  of characters the service gives you when you sign up. It is charged to your
  account, so treat it like a password: keep it out of screenshots and out of
  chat messages.
- The four values for the file `.env.example` that is in this zip. It is a
  template with empty values, so nothing works until you fill them in.

## Running it

1. Unpack the zip into a folder, for example `C:\Lumingo`. Do not run the
   program from inside the zip; unpack it first.
2. In that folder, copy `.env.example` and rename the copy `.env`. Windows
   hides file extensions, so turn on "File name extensions" in File Explorer
   first. The name must be exactly `.env`.
3. Open `.env` in a text editor, such as Notepad, and fill in four lines:
   - `TUTOR_LLM_PROTOCOL` — write `openai_chat` or `anthropic_messages`.
   - `TUTOR_LLM_BASE_URL` — the address of the service, exactly as the
     service's own page shows it.
   - `TUTOR_LLM_MODEL` — the name of the model your service offers.
   - `TUTOR_LLM_API_KEY` — your key. It belongs in this file on your own
     computer and nowhere else.
   Save the file.
4. Double-click `lumingo.exe`. A black window opens, it prints the address it
   is using, and your browser opens on that address. The line looks like
   `Lumingo is running at http://127.0.0.1:8765`.
5. If the port `8765` is already taken on your computer, the program takes
   the next free one and prints that address instead. Use the address it
   prints, not the one above.
6. To stop the program, close the black window.

If the browser does not open by itself, copy the address from the black window
and paste it into your browser.

## The Windows SmartScreen warning

Windows may show a blue SmartScreen warning the first time you run this program. SmartScreen warns about programs it has not seen before, and this program is unsigned, so Windows cannot check who published it. Choose "More info" and then "Run anyway" to start it. The files in this zip can be checked against the SHA-256 sums in checksums.txt.

The warning does not say that something is wrong with the program. It says
that Windows has not seen this program before and cannot check who made it.

`checksums.txt` is next to the zip in your download folder, not inside it. It
holds one SHA-256 sum for each of the five files. To check them yourself,
open PowerShell in the unpacked folder and run:

```powershell
Get-FileHash -Algorithm SHA256 .env.example, LICENSE, NOTICE, README.md, lumingo.exe
```

## What is in this zip, and what is not

The zip holds five files:

- `lumingo.exe` — the program.
- `LICENSE` — the licence of the code.
- `NOTICE` — the notices that have to travel with the code.
- `.env.example` — the template for your four values. It is empty on purpose.
- `README.md` — this file.

Four things are not in the zip, on purpose:

- **The microphone and the speech engines.** This build has no audio features
  enabled, so the speaking parts do not work.
- **The curriculum.** No units are in this zip, so you cannot finish a unit
  with it. What you can do is check that the program runs and set it up.
- **The speech models.** The list of them is not in this zip. The program
  looks for `models/manifest.toml` next to `lumingo.exe`, and this build has
  none, so the speech-model list in Settings cannot be loaded and there is
  nothing for the program to fetch.
- **A signature.** The program is not code-signed. That is why Windows shows
  the SmartScreen warning.

## Where your data lives

Your history, your settings and the service you set up are stored on this
computer, in the folder `%LOCALAPPDATA%\Lumingo`, for example
`C:\Users\YourName\AppData\Local\Lumingo`. It holds:

- `lumingo.sqlite` — your history, your attempts and your settings.
- `providers.toml` — only if you add a service inside the program. It holds
  that service's key in plain text.
- Recordings — only if you turn recordings on in the settings. They stay in
  this folder too.

The `.env` file stays in the folder you unpacked the zip into. It holds your
key in plain text, on your own disk. The key is sent only to the service you
configured.

The program listens on `127.0.0.1` only, so nothing else on your network can
open it. To remove your history, close the program, then delete the
`%LOCALAPPDATA%\Lumingo` folder and the `.env` file.
