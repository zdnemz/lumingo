# Installing Lumingo

This guide is for someone who has never set up an AI service before. It
covers the Windows build: one zip, five files, nothing to compile. By the end
of it, the program runs on your own computer and opens in your browser.

If you want to know first what this build can and cannot do, read
[LIMITATIONS.md](LIMITATIONS.md).

## What you need

- A Windows 11 machine, 64-bit, with about 8 GB of RAM.
- A current browser, and an internet connection, so the program can reach the
  provider you choose.
- A microphone and speakers, if you want to talk to the tutor. This build has
  no microphone support and no speech engines, so the speaking steps do not
  run here. The last section explains that in full.
- An API key from a provider that speaks the OpenAI-compatible or the
  Anthropic-compatible message format.

An API key is a long string of characters that a provider gives you when you
sign up, and it is what lets Lumingo ask that provider for an answer; the use
is charged to your account, so treat it like a password and keep it out of
screenshots and out of chat messages. Providers often give a free allowance
for some of their models, so you may be able to try the program before you
pay anything. What is free, and for how long, is set by the provider, not by
Lumingo.

You also need some free disk space. The program writes its own data to
`%LOCALAPPDATA%\Lumingo` on your computer.

## Unpack the zip

The download is `lumingo-1.0.0-windows-x64.zip`. Unpack it before you do
anything else: do not run the program from inside the zip.

The zip holds exactly five files, and nothing else:

- `lumingo.exe` — the program itself.
- `.env.example` — an empty template with the four settings you will fill in.
- `README.md` — the short guide that ships with the program.
- `LICENSE` — the licence of the code.
- `NOTICE` — the notices that have to travel with the code.

Unpack it into a folder you can write to, for example `C:\Lumingo`. The
program keeps its data in `%LOCALAPPDATA%\Lumingo`, but the `.env` file with
your key stays next to `lumingo.exe`, so the folder has to be one you can
write to. A folder inside `Program Files` is a bad idea: Windows protects
that folder, and the program cannot create the files it needs there.

## Set up your provider

In the unpacked folder, copy `.env.example` and rename the copy `.env`. The
new name must be exactly `.env`. Windows hides file extensions by default, so
turn on "File name extensions" in File Explorer first, or the copy ends up
named `.env.example.txt` and the program will not find it.

Open `.env` in a text editor such as Notepad. It is a template with four
empty settings, and nothing works until you fill in all four:

- `TUTOR_LLM_PROTOCOL` — which of the two message formats your provider
  speaks. Lumingo accepts only two words here, the OpenAI-style chat format
  and the Anthropic-style messages format, and it says so in the error it
  prints when the setting holds anything else. The provider's own page tells
  you which format it uses.
- `TUTOR_LLM_BASE_URL` — the address of the provider's service, copied
  exactly as the provider's own page shows it.
- `TUTOR_LLM_MODEL` — the name of one model that the provider offers.
- `TUTOR_LLM_API_KEY` — your key from the provider.

Save the file.

The key in `.env` is saved in plain text, on your own computer, in the folder
you unpacked the zip into. Lumingo sends it only to the base URL of the
profile you have set up, and nowhere else. Do not put the key in a
screenshot, in a chat message, or in any file you hand to somebody else. What
else leaves your computer, and what does not, is written out in
[PRIVACY.md](PRIVACY.md).

## Start the program

Double-click `lumingo.exe`. A black window opens, it prints the address it
is actually using, and your browser opens on that address. The line normally
looks like this:

```
Lumingo is running at http://127.0.0.1:8765
```

`127.0.0.1` is your own computer, so nothing else on your network can open
the program.

If the port `8765` is already taken on your machine, the program takes the
next free one and prints that address instead. Use the address it prints, not
the one above. If the browser does not open by itself, copy the address out of
the black window and paste it into your browser. To stop the program, close
the black window.

**The SmartScreen warning.** The first time you start the program, Windows
may show a blue SmartScreen warning. This build is **unsigned**, so Windows
has no signature to check and warns about programs it has not seen before.
Choose "More info", then "Run anyway", to start it.

The warning is about the missing signature, not about your download. It does
not say that something is wrong with the program or that the files were
changed. The sums in the next section tell you whether the files on your disk
are the ones that were published.

## Check the files

Next to the zip in your download folder there is a file called
`checksums.txt`. It is not inside the zip. It is in the `sha256sum` format:
one line per member of the zip, and each line holds the SHA-256 sum of that
file together with its name. There are five lines, one for each of the five
files.

Check the sums before you run the program. Windows has a checksum tool built
in: open PowerShell in the unpacked folder and run

```powershell
Get-FileHash -Algorithm SHA256 .env.example, LICENSE, NOTICE, README.md, lumingo.exe
```

and compare each sum with the matching line in `checksums.txt`. If you copy
`checksums.txt` into the unpacked folder and you have a `sha256sum` tool,
the same check is one command:

```
sha256sum -c checksums.txt
```

Every line has to say `OK`. A mismatch means the file on your disk is not the
file that was published: the download is damaged, or the file was changed
after you unpacked it. Do not run the program. Unpack the zip again and
check once more, and if the sums still do not match, do not use the
download.

## What is not in this build

Four things are missing on purpose. They are known limits of this release,
not a broken download:

- **No curriculum.** The zip has no units in it, so the unit list starts
  empty and no unit can be completed from this artifact. What you can do is
  set the program up and see that it runs.
- **No microphone support.** The default build does not open a microphone.
- **No speech engines.** The default build ships no speech engines, so the
  speaking steps cannot be run in it. The text steps — chat, lessons, reading
  and writing — work.
- **No signature.** The program is not code-signed, which is why Windows
  shows the SmartScreen warning.

The full list is in [LIMITATIONS.md](LIMITATIONS.md) and
[RELEASE_README.md](RELEASE_README.md).
