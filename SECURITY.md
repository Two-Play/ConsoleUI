# Security policy

ConsoleUI is pre-alpha. Only the latest `main` is supported.

## Reporting a vulnerability

Please do **not** open a public issue. Use GitHub's
[private vulnerability reporting](https://github.com/Two-Play/ConsoleUI/security/advisories/new)
and include steps to reproduce, the affected component (shell, compositor, session scripts) and your environment.

We aim to answer within a week. Areas that matter most: the compositor control socket, the
shell's exclusive input grab, launching of processes, and anything that runs with extra privileges
(session installation).
