# Historical synthetic wake test fixture

`hey-groq.rpw` is a Rustpotter 3 reference model containing MFCC features, not
an audio recording or a neural network. Phrase: **Hey Groq** (pronounced
“Hey Grok”). Its three reference samples were synthesized locally using
eSpeak NG 1.52.0, converted to 16 kHz mono PCM16 using FFmpeg, then passed to
`whispertype train` with the default 16 MFCC coefficients.

Reproduction parameters (eSpeak text is `Hey Grok`):

| Voice | Speed | Pitch |
|---|---:|---:|
| en-us | 155 | 45 |
| en-gb | 170 | 55 |
| en-us+f3 | 150 | 62 |

The positive test fixture is a held-out eSpeak `en-gb-scotland` voice at speed
160 and pitch 48. The negative fixture is the same voice saying “Good
morning, this is a routine sentence.” at speed 160 and default pitch.
`test-positive.pcm` and `test-negative.pcm` contain raw little-endian PCM16
mono at 16 kHz, without a WAV header; they are used only in tests and are not
embedded into the release executable.

Synthetic tests are a regression check, not evidence of real-world accuracy.
This model is now used only by tests/benchmarks. Production requires a personal
model and never falls back to this synthetic reference.
Use the tray's personal phrase enrollment for your own voice. eSpeak and
FFmpeg are build-time generation tools and are not shipped or required by
the application. Generated model/fixtures are supplied under the project
license.
