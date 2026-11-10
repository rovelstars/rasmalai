---
title: Packaging
description: Declare an app with AppConfig and write its app.json manifest with AppBundle.
section: Packaging
order: 90
---

# Packaging

An app has a name, a version, an entry file, and a list of capabilities. Keep
that description separate from the running window, then write it as the
`app.json` manifest when a tool needs it.

`AppConfig` holds the title, size, appearance, and entry. `AppConfig.defaults`
fills in 760 by 560, `Appearance.system`, and `src/main.rnx`:

```rnx
import { AppBundle, AppConfig } from "@std/app"

fn Main(): Int {
    let config = AppConfig.defaults("Notes")
    println!(config.titleText())
    println!(config.widthValue())
    println!(config.heightValue())

    let bundle = new AppBundle("notes", "0.1.0", "src/main.rnx", ["fs:delegated"])
    println!(bundle.manifest())
    if !bundle.writeTo("app.json") {
        return 1
    }
    return 0
}
```

`bundle.manifest()` returns one JSON object with `name`, `version`, `entry`, and
`capabilities`. `bundle.writeTo(path)` writes it with overwrite semantics and
returns true on success. `AppBundle` writes the manifest only. It does not collect
entry files or create an archive.

## Appearance

`AppConfig.theme(systemDark)` resolves `Appearance.system` against a desktop
preference. `Appearance.light` and `Appearance.dark` ignore the argument:

```rnx
import { AppConfig, Appearance } from "@std/app"

let config = new AppConfig("Notes", 800, 600, Appearance.system, "src/main.rnx")
let light = config.theme(false)
let dark = config.theme(true)
println!(light.background != dark.background)
```

Use the resolved theme with `new App(window, theme, view)` when the app should
follow the desktop. `App.launchView` and `App.launch` start on the dark theme.

## Capabilities

The capability list in `AppBundle` is copied into the manifest as strings. It
should name what the app actually needs, such as `fs:delegated` for delegated
file access or a scoped `fs:read:*` grant. Keep it in step with the
`permissions` entry in `Project.config`.

`rnx lock` records the approved capability ledger, and `rnx run --locked` and
`rnx build --locked` verify it before running or building. A new capability in
the code fails the locked build until the ledger is updated, so the manifest is
not the only place that grants access.

## Build and run

```sh
rnx build
rnx run --backend cranelift
```

`rnx build` links a native Linux executable. Native UI apps still need the
wrapper libraries described in [Native wrappers](native-wrappers), and the
interpreter cannot pass `Float` to a foreign call, so use a compiled backend for
a running app.

## Keep going

Return to the [overview](../overview) for the full type map, or read
[Testing](testing) to check the app before packaging it.
