# RBXFoundry examples

## Castle (`make_examples.sh`)

```bash
./scripts/build.sh                 # or use a release tarball's bin/
examples/make_examples.sh          # writes castle.rbxl, castle.png, ui.png
```

Builds a small castle place entirely through the CLI: two towers on a
baseplate, a `SpawnLocation`, a StarterGui HUD (including RichText),
and a Luau day/night `Script`. It then renders a genuine 3D screenshot,
a UI screenshot, and validates the place.

Open it with any RBXFoundry command, e.g.:

```bash
blox tree examples/castle.rbxl
blox play start examples/castle.rbxl
```

Everything is offline; no Roblox account is involved. See
[../README.md](../README.md) and [../COMPATIBILITY.md](../COMPATIBILITY.md)
for what is and is not supported.
