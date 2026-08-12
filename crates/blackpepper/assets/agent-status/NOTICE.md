# Agent-status blocker manifests

The blocker phrases and region strategy in these manifests were adapted from
Herdr's Apache-2.0 agent-detection manifests at commit
`06ca0baa12f4203c5bbad9ecadf53f9a475a52b2` (2026-08-11):

https://github.com/herdrdev/herdr/tree/06ca0baa12f4203c5bbad9ecadf53f9a475a52b2/src/detect/manifests

Blackpepper intentionally keeps only rules that identify a visible request for
human input. These files cannot declare working, idle, done, or exited state.

A copy of Herdr's Apache License 2.0 is provided in
`LICENSE-HERDR-APACHE-2.0` and in every Blackpepper release bundle.
