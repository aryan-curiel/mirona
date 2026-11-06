# Security and privacy issues

Mirona records private conversations, so we treat privacy bugs as security bugs. Examples:

- anything leaving the Mac while privacy mode is on
- a sensitive line (password, salary, account number…) reaching a summary, a search vector, a
  cloud model other than the classifier tagging it, or an MCP result that didn't ask for
  `include_sensitive`
- encrypted meeting data readable without the key
- an MCP client reading meetings without a valid token, or reaching the server from another
  machine

## Reporting

Please **don't open a public issue**. Report it privately through GitHub:
[Security › Report a vulnerability](https://github.com/aryan-curiel/mirona/security/advisories/new).

Include what you did, what you expected, and what happened. Logs, a network capture or a short
recording help. Don't include real meeting content.

We'll reply within 7 days, keep you updated, and credit you in the release notes unless you
prefer not to be named.

## Supported versions

Only the latest release gets fixes.
