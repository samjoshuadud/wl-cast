# Security policy

wl-cast streams a screen and injects keyboard and mouse input, so a bug here can let someone control a computer. Please report problems privately.

## Reporting a vulnerability

Use GitHub private vulnerability reporting:

https://github.com/samjoshuadud/wl-cast/security/advisories/new

Do not open a public issue for a vulnerability. Include what you found, how to reproduce it, and the compositor and wl-cast version you used.

This is a small project. We aim to reply within a week and will keep you updated on the fix.

## Supported versions

Only the latest 0.1.x release gets security fixes.

## Scope

In scope: the library code (`Capture`, `Encoder`, `Input`, `RtpSender`, `RemoteSession`, `AccessToken`).

The programs in `examples` are demos. Weaknesses in them are welcome as normal issues, unless they show a flaw in the library itself.

## Known limits

These are documented, not vulnerabilities:

- Anyone with the secret link can see and control the screen. Treat it like a password.
- The example web page uses plain HTTP, so the link can be read on an untrusted network. Use a VPN or an HTTPS proxy.
- The example web server can be stalled by one slow client.
