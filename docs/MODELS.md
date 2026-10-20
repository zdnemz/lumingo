# Models

Lumingo keeps a list of the speech models it expects to use: the ones that
listen to the learner, turn speech into text, speak back, and score
pronunciation. The list is [`models/manifest.toml`](../models/manifest.toml).
It is the only source of download locations; no other file in the repository
names one.

None of them is installed. Every entry in the manifest is a candidate, and the
section *Why nothing can be downloaded yet* below says what stops each one.

## What the models are for

There are four roles. The `role` field of a manifest entry is one of them, and
each role is a separate engine with its own files.

**Voice activity detection (`vad`).** The `Vad` trait in the `speech` crate
consumes one frame of 16 kHz mono audio and returns the probability that the
frame holds speech. It is what tells the program that the learner has started
and that the learner has stopped, so an utterance has a beginning and an end
and the microphone is not left open between them.

**Speech-to-text (`stt`).** The `SttEngine` trait turns the learner's audio
into a `Transcript` of words with their timings. That transcript is what the
tutor's analysis and the practice scoring read: the words the learner actually
produced, and when each one landed.

**Text-to-speech (`tts`).** The `TtsEngine` trait reads text and streams PCM
chunks back, so the tutor's lines and the sentences of a lesson are spoken
aloud instead of only being shown.

**Pronunciation scoring (`pron`).** The `pron-engine` crate turns a reference
text into phonemes, forces an alignment against the posteriors of a phoneme
recogniser, and scores the result. The model behind it is the one this role
installs; the rest of that crate is plain functions over plain data and is
tested without a model.

These four are the speech models, and they are a different thing from the
tutor's own text. The tutor's text — the scenario, the learner's message, the
tutor's reply — is sent to the AI provider the user has chosen, over that
provider's own connection. Nothing on this page is involved in that. The
models listed here would sit on the learner's machine and do the listening,
the speaking and the scoring; the provider does the talking.

## The candidate models

Ten entries, all of them candidates. The ids, the roles, the engines and the
licences are copied from the manifest; the `version` is `candidate` and the
recorded size is `0` for every row, because nothing has been measured yet.

| Id | Role | Engine | Licence | Version | Source |
|---|---|---|---|---|---|
| `vad-silero` | `vad` | `sherpa-onnx` | MIT | candidate | https://github.com/snakers4/silero-vad |
| `stt-whisper-base-en` | `stt` | `sherpa-onnx` | MIT | candidate | https://huggingface.co/openai/whisper-base.en |
| `stt-whisper-small-en` | `stt` | `sherpa-onnx` | MIT | candidate | https://huggingface.co/openai/whisper-small.en |
| `stt-moonshine-english` | `stt` | `sherpa-onnx` | MIT | candidate | https://github.com/moonshine-ai/moonshine |
| `stt-parakeet-tdt` | `stt` | `sherpa-onnx` | CC-BY-4.0 | candidate | https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2 |
| `tts-supertonic-3` | `tts` | `sherpa-onnx` | OpenRAIL-M | candidate | none recorded in the manifest |
| `tts-kokoro` | `tts` | `sherpa-onnx` | Apache-2.0 | candidate | https://huggingface.co/hexgrad/Kokoro-82M |
| `tts-kitten` | `tts` | `sherpa-onnx` | Apache-2.0 | candidate | https://github.com/KittenML/KittenTTS |
| `pron-wav2vec2-lv-60-espeak` | `pron` | `ort` | Apache-2.0 | candidate | https://huggingface.co/facebook/wav2vec2-lv-60-espeak-cv-ft |
| `pron-zipa-small` | `pron` | `ort` | unknown | candidate | none recorded in the manifest |

Two rows have no source: `tts-supertonic-3` and `pron-zipa-small` have an
empty `source` in the manifest, so nothing is written in their place. An
entry whose `source` is a project page rather than a file directory still
cannot be fetched: the manifest's own note says the source is the upstream
project or model page, "not yet a directory the files can be fetched from".

Each entry also carries a `notes` field naming what is still unverified. They
are copied from the manifest:

- `vad-silero`: which ONNX file sherpa-onnx loads, and where it is published.
- `stt-whisper-base-en`: the source is the upstream model card; sherpa-onnx
  loads an ONNX export of it, so the export host and the file names are still
  to be confirmed.
- `stt-whisper-small-en`: the same open point as base.en about the ONNX
  export.
- `stt-moonshine-english`: English models only, because the other languages
  are non-commercial and are excluded in the register; which size to use and
  whether sherpa-onnx can run it are decided in stage 1.
- `stt-parakeet-tdt`: the model card and the licence are to be confirmed, and
  attribution is required, so a NOTICE line has to be added when it is chosen.
- `tts-supertonic-3`: the use restrictions of OpenRAIL-M must be read in full
  and put in `license_text` before this entry is downloadable; the `license_url`
  and the `source` are empty because nothing has been checked at the source.
- `tts-kokoro`: the usual text front end needs espeak-ng, which is GPL and
  excluded, so the entry stays empty until it is confirmed how sherpa-onnx
  packages the front end.
- `tts-kitten`: the upstream location of the ONNX files, and that the model
  licence is Apache-2.0.
- `pron-wav2vec2-lv-60-espeak`: the licence on the model card and the ONNX
  export are to be confirmed.
- `pron-zipa-small`: the licence and the ONNX export are both unverified, so
  the licence stays `unknown` and the entry stays refused.

The manifest fields are described field by field in
[`models/README.md`](../models/README.md).

## Download hosts

The `source` URLs of the entries above point at two hosts: `github.com` and
`huggingface.co`. `github.com` carries the Silero VAD page, the Moonshine
repository and the Kitten TTS repository; `huggingface.co` carries the Whisper
model cards, the Parakeet model card, the Kokoro model card and the wav2vec2
model card. The two entries with no source name no host at all.

Those two are the only hosts a model download would ever go to. The downloader
takes the URL from the manifest and nothing else: it refuses a URL whose
scheme is not `https` (plain `http` is accepted for the loopback address
only), it refuses a URL that carries credentials, and it follows no location
that the manifest does not name. There is no fallback host, no mirror and no
other address in the code.

A provider's own host is a separate thing and is not in that list. When the
tutor's text goes to the AI provider the user has chosen, that request goes to
the host that provider profile names, and the models above have nothing to do
with it. See [PRIVACY.md](PRIVACY.md) for what leaves the machine and where.

## Why nothing can be downloaded yet

The short version: no model can be downloaded yet.

Every entry in the manifest is a `candidate`, every entry has an empty `files`
list, and no file has a checksum recorded. There is nothing to fetch and
nothing to verify, and the code refuses each entry before it opens a
connection. `ModelEntry::check_downloadable` returns an error for an entry
with no files, and `ModelManager::install` calls it first and stops there. The
same check refuses a file whose `sha256` is empty or is not 64 hexadecimal
characters, an entry with no licence or no licence URL, and an entry whose
licence is the word `unknown`. `model-manager verify` lists the entries that
are refused and why.

The manifest is the only source of download locations. Nothing else in the
repository names a file, a host or a path to a model, and there is no bundled
copy to fall back on: no model file is stored in this repository or in the
release zip.

No model is installed on any machine from this repository. Not on the machine
that built it, not on the machine that committed it, not on a learner's
machine. This is a known limit, recorded in [LIMITATIONS.md](LIMITATIONS.md),
and not a bug to be worked around. There is no supported way to make an entry
downloadable from outside the manifest, and the downloader will not do it.

The way out is the way the manifest describes: the owner downloads each file
from the upstream host, checks the licence at the source, computes the SHA-256
of the file they downloaded, and fills in `files`, `size_bytes`, `version`,
`license_url` and `license_text`. A checksum copied from anywhere else does
not count. Until that has happened for an entry, it stays a candidate, and the
whole manifest stays a list of what Lumingo expects to use.

## Licences

Each model carries its own licence. The manifest has a `license` field and a
`license_url` field per entry, and the two go together: an entry whose licence
is empty, or is the word `unknown`, or whose licence URL is empty, is refused
by `check_downloadable` before any request. `tts-supertonic-3` is marked
`license_review = true`, because OpenRAIL-M carries use restrictions that have
to be read in full.

The licences are recorded in `docs/LICENSE_REGISTER.md`, section 6, with the
status they are still at. The rows there are what the register expected to
find; they are not what anyone has read at the source yet, and each one stays
`verify` until someone reads the licence at the source.

A download needs two things, in this order.

1. The licence notice for that model is shown and accepted. The notice carries
   the model id, the licence name, the licence URL and, when the manifest has
   it, the licence text or the use restrictions in full. An acceptance only
   counts for the same model, the same licence name and the same licence URL
   the entry carries at that moment.
2. The file is verified against the checksum recorded for it. The download
   goes to `<file>.part`, is resumed with `Range: bytes=N-` if it is
   interrupted, and is moved into place only when its SHA-256 matches the
   `sha256` in the manifest. A mismatch keeps nothing.

Nothing is fetched before the notice has been accepted, and nothing is kept
before the SHA-256 matches. The manifest says a licence with attribution
needs a NOTICE line, and `docs/LICENSE_REGISTER.md` says the same.
