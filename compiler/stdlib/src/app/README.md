# @std/app

Build windows, frames, input, routing, and tests with @std/app.

```rnx
import { App } from "@std/app"
import { Page, Text } from "@std/ui"

fn Main(): Int {
    let app = App.launchView("Hello", 320, 240, () => Page(title: "Hello") {
        Text("Hello from @std/app")
    })
    return app.show()
}
```

Read the [overview](/docs/@std/app/overview) or open the [API reference](/docs/@std/app/api).
