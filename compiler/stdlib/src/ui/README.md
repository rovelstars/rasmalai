# @std/ui

Build a screen as a View value, then mount it with @std/app.

```rnx
import { Page, Text, TextEngine, Tree, View } from "@std/ui"

fn Main(): Int {
    let tree = new Tree(TextEngine.fixed())
    tree.beginFrame()
    let root = View.mount(tree, Page(title: "Today", padding: 20, gap: 12) {
        Text("3 open")
        Text("Write the docs")
    })
    tree.endFrame()
    tree.layout(root, 320)
    println!(tree.snapshot())
    return 0
}
```

Read the [overview](/docs/@std/ui/overview) or open the [API reference](/docs/@std/ui/api).
