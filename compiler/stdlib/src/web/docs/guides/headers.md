---
title: "Headers and status codes"
description: Store case-insensitive headers and convert between status variants and numeric codes.
section: "HTTP"
order: 40
---

# Headers and status codes

HTTP header names are case-insensitive, and a status code is a number with a
name attached. Treating either as a plain string leads to duplicate entries
and string comparisons. `Headers` and `HttpStatus` give both a stable shape.

## Store headers by name

```rnx
import { Headers } from "@std/web";

let h = Headers.fromPairs([
    ["Content-Type", "text/plain"],
    ["X-Request-Id", "abc"]
]);

println!(h.len(), h.keys().join(","));
println!(h.get("CONTENT-TYPE") ?? "missing");

h.set("content-type", "text/html");
println!(h.get("Content-Type") ?? "");
println!(h.delete("x-request-id"), h.has("X-Request-Id"));
```

```text
2 content-type,x-request-id
text/plain
text/html
true false
```

Every name is folded to lowercase on the way in, so each lookup, update, and
delete uses the same entry. `get()` returns null when the name is absent and
`""` when the header exists with an empty value. `keys()` and `values()` keep
insertion order and align by index. `set()` on an existing name overwrites the
value without moving it; `delete()` reports whether anything was removed.

Build a table from an existing `Map` with `Headers.fromMap()`, or from an
array of pairs with `Headers.fromPairs()`, which reads the first two elements
of each pair and skips shorter ones.

## Convert a status code

`HttpStatus` names 17 common codes. `statusCode()` gives the number,
`statusReason()` gives the phrase, and `statusFromCode()` goes back the other
way. That reverse lookup returns null for a code the enum does not name.

```rnx
import {
    HttpStatus,
    statusCode,
    statusReason,
    statusFromCode,
    isSuccess,
    isClientError,
    isServerError
} from "@std/web";

println!(statusCode(HttpStatus.NotFound), statusReason(HttpStatus.NotFound));
println!(statusFromCode(201) == HttpStatus.Created);
println!(statusFromCode(418) == null);
println!(isSuccess(204), isClientError(404), isServerError(503));
```

```text
404 Not Found
true
true
true true true
```

`isSuccess()`, `isClientError()`, and `isServerError()` take a number, not an
`HttpStatus`, and work on codes the enum does not name. Use them when a
response carries an uncommon code.

A `Response` stores the status line as a number and a reason string. Feed
`response.status` to `statusFromCode()` when you want the variant, or to the
range helpers when you only need the class of result.
