Courtesy of [@jmickelin](https://github.com/jmickelin): https://github.com/jmickelin/multi-home-cabal-example

# Minimal workable example of a multi-home Cabal project

This repository serves as a minimal example to illustrate the bug
fossas/fossa-cli#829.

The project consists of a `cabal.project` file at the top level, which
refers to several libraries and services in subdirectories. It also
lists an external git dependency.

No `.cabal`-file exists at the top level.

Running `cabal v2-build` on the top level gives the following error
message:

```
cabal: No targets given and there is no package in the current directory. Use
the target 'all' for all packages in the project or specify packages or
components by name or location. See 'cabal build --help' for more details on
target options.
```

Instead, you have to run `cabal v2-build all`.

Running `cabal v2-build` from within `lib/awesome-prelude` or
`lib/gnarly-library` does work, but running it from within
`service/funky-service` again requires specifying a target (e.g.
`all`). This is because the service has multiple targets (a library,
an executable and a test suite).
