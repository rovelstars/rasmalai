---
title: "Collection Blocks"
description: "A trailing block that supplies an array argument, with control flow, slots, and spread."
section: "Syntax and Primitives"
icon: "Braces"
---

# Collection Blocks

A call can end with a block. When the callee's final parameter is an array,
the block supplies that parameter. This is a general language feature, not a
UI feature.

```rnx
Page(padding: 20) {
    Heading("Today")
    Text("Two open tasks")
}
```

is the same call as:

```rnx
Page(padding: 20, children: [
    Heading("Today"),
    Text("Two open tasks"),
])
```

When the callee's final parameter is not an array, the older child-sink block
form remains for builder compatibility. New library APIs should take an array
parameter.

## Elements

- Each expression statement contributes one element, in order.
- `if`, `else`, `for`, and `while` contribute the elements of the branch or
  iteration they take. `switch` arms follow the normal switch rules; each arm
  still needs its explicit terminator.
- `let` and `const` are scoped to the block and contribute nothing.
- `return` cannot leave a collection block. It is `E108`.
- A throwing call, a throwing constructor, or `?` cannot appear in a collection
  block. Hoist it above the block or handle it outside. All are `E108`.
- `break` and `continue` may target a loop inside the block. Leaving the
  collection block with either is `E108`.
- `...items` splices an existing array into the current collection.
- `name: { ... }` supplies a named array parameter. Braces are required.

```rnx
List {
    Header {
        Text(openCount.toString() + " open")
        Spacer()
        Button("Clear", clear)
    }
    for task in tasks {
        TaskRow(task)
    }
}
```

## Slots

A slot is a named array parameter. It is useful when a component has more than
one region.

```rnx
Sheet(detents: [0.0, 280.0, 520.0]) {
    title: { Heading("Details") }
    Text("Body")
    actions: {
        Button("Close", close)
        Button("Save", save)
    }
}
```

The unnamed statements fill `children`. `title:` and `actions:` fill those
parameters, which must also be arrays. Duplicate slot names and passing the
same parameter in the call head and the block are `E108`.

## Type rules

- The callee's final parameter must be `Array<T>`.
- Every element must have type `T`. `Any` is accepted only for `Array<Any>`.
- A spread value must be `Array<T>`.
- A slot's element type comes from that slot's parameter, not from `children`.
- A slot that does not name an array parameter is `E108`.
- Type errors name the actual and expected element types.

## Current limits

A method chain after a trailing block and a nested trailing block both parse.
Collection-block detection covers free functions, static factories, instance
methods on a typed local, and `this.method`. Generic array parameters are
rejected until callee type-argument substitution lands.

## For library authors

Declare the array parameter last and give it a default:

```rnx
fn Stack(
    gap: Int = 8,
    children: Array<View> = [],
): View
```

Callers can then write `Stack { ... }` or `Stack(gap: 12) { ... }`. The block
is evaluated in the caller's scope. There is no receiver and no hidden builder
state. The callee receives a normal array.
