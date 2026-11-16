# Slice 1 experiment decisions

## One long-lived process per loaded slot

Load each explicitly selected binary/ID in its own worker, using pinned `clap-sys` ABI bindings and `libloading`.
Keep the parent free of plugin code.
This is the smallest route that tests real CLAP lifecycle/processing and contains crashes without introducing a general host or UI dependency.
Additional workers act only as same-plugin history controls.
This is availability isolation, not a security sandbox.

Use the worker's main OS thread for main-thread calls and temporarily mark it as the symbolic audio thread for serialized audio calls.
CLAP's thread-check specification explicitly permits this offline arrangement.
Service queued main-thread callbacks between blocks and every 10 ms while idle.

## Compare matched histories, not merely matched parameter values

CHOWTape reset/preconditioning trials differed, while fresh instances with the same full command history matched exactly.
Mirror its preceding trials in the control before testing cross-slot edits/reset.
Do not erase the observed reset limitation by raising the tolerance, disabling reference processing, or substituting a plugin/format.
A fresh loaded worker reproduced the first tested trial, but this experiment does not choose a future application's reset policy or claim complete state restoration.
