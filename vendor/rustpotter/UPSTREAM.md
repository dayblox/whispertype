# Local resource-efficiency changes

Based on Rustpotter 3.0.2, upstream commit
`46bbc73223afe5fc7b90f30455046432fc3c9432`.
Upstream: https://github.com/GiviMAD/rustpotter
License: Apache-2.0; the original LICENSE is retained.

Whispertype modifications:

- Reuse two DTW rows instead of allocating and copying full cost matrices.
- Preserve the upstream terminal-cell indexing exactly for score compatibility.
- Compute each vector's squared magnitude once per comparison, preserving
  the original accumulation order and square-root arithmetic.
- Borrow live MFCC windows and reuse normalization buffers by reference length.
- Avoid building a sample-score map unless a candidate passes the threshold.
- Cache FFT plans, scratch storage, spectra and DCT weights; reuse audio,
  pre-emphasis and mel-cepstrum workspaces without changing arithmetic order.
- Apply formatting and equivalent current-Clippy cleanups to the vendored
  source. `mfcc/extractor_reference.rs` retains the upstream extraction
  algorithm as a test-only oracle.

No threshold, MFCC extraction, feature normalization arithmetic, or model
format change is intended. Reference-comparison tests and whole-recording
equivalence checks guard the optimized path.
