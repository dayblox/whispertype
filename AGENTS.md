# Project instructions

## Resource efficiency is a required constraint

The user wants Whispertype to use as little RAM and CPU as physically practical
in **every state**: idle, wake listening, recording, transcription, forms,
update checks, installation, and shutdown. Preserve this requirement in future
threads. Do not trade efficiency for implementation convenience.

- Do not introduce a known RAM or CPU regression. If a requested feature cannot
  avoid one, explain the measured cost and obtain the user's explicit agreement
  before accepting that tradeoff. Never claim physical minimality without evidence.
- Preserve event-driven idle behavior: no periodic polling, animation loops,
  background network checks, idle microphone capture, or timers without a real
  active operation. Close microphones, overlays, helpers, and other resources as
  soon as their work ends.
- Stream uploads/downloads and reuse bounded buffers. Avoid duplicate audio or
  executable buffers, unbounded queues, per-frame/per-callback allocations, and
  unnecessary threads, processes, dependencies, or heavyweight UI runtimes.
- Keep transient network/TLS work in short-lived helpers so its allocations are
  reclaimed on exit. Keep forms native and process input only when events arrive.
- Before changing a performance-sensitive path, inspect `OPTIMIZATION.md`,
  `VALIDATION.md`, and the relevant existing benchmarks. Measure comparable
  before/after RAM and CPU behavior for affected states, including idle when a
  change can affect it. Run relevant allocation/equivalence checks. Record the
  conditions and limits of the evidence; do not infer improvements from code
  shape alone. Correct regressions before completing the change.
- Preserve correctness, input delivery, wake accuracy, bounded recording, and
  user configuration while optimizing. Do not hide costs through working-set
  trimming or other measurement-only tricks.
