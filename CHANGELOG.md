# Changelog

All notable changes to scaler are documented in this file.

## 0.2.0

### Fixed

- `scaler run --cpu` and `--mem` now actually enforce limits on Linux via a
  transient `systemd --user --scope` unit. Previous releases accepted the
  flags but launched the command unconstrained because the run loop was
  hardcoded to a fallback backend.
- macOS runs now wrap the target with `taskpolicy -b -d throttle -g default`
  for best-effort CPU lowering. The `-m` memory flag is now gated on whether
  the host's `taskpolicy` actually supports it, instead of being emitted
  unconditionally and crashing at spawn time.
- `scaler` warns to stderr when the user passes `--cpu` or `--mem` but the
  effective backend is `plain_fallback`, so users can no longer be silently
  unenforced.

### Added

- `scaler doctor` now prints an `effective_backend:` line that names the
  backend `scaler run` will actually use (`linux_systemd`, `macos_taskpolicy`,
  or `plain_fallback`).
- `Backend::sample` now aggregates RSS and CPU across the entire descendant
  process tree of the launched command, not just the root pid.
- Release tarballs now ship `aarch64-unknown-linux-gnu` (Linux ARM64)
  alongside `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`. CI now
  exercises Linux ARM64 too via the `ubuntu-24.04-arm` runner.
- New `SCALER_FORCE_BACKEND` test escape hatch (intended for integration
  tests, not user-facing API).

## 0.1.0

- Initial release: CLI, doctor, run loop, monitor, plain fallback.
