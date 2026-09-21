# Security policy

## Supported versions

The latest released minor version receives security fixes.

## Reporting

Do not open a public issue for a suspected vulnerability. Use GitHub private vulnerability reporting for this repository.

Include the affected version, operating system, input shape, impact, and a minimal reproduction. Do not include confidential source code.

The maintainers will acknowledge a complete report within seven days. They will coordinate validation, remediation, and disclosure with the reporter.

## Input trust

`leadline` analyzes untrusted source and coverage files. Run it with the same file and process permissions as other build tools.

Commands that read Git history execute the installed `git` binary with fixed arguments. They do not execute repository hooks or source code.
