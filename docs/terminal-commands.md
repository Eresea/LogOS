# LogOS Flow

The terminal submits bounded Flow source to the Flow service. Flow parses, type-checks, and
evaluates the source against the typed system-operation registry.

The terminal context provides bounded help without reintroducing the legacy command parser:

```ts
help()
help("fs")
clear()
echo("hello")
```

The first form lists the typed Flow namespaces; the second shows a namespace summary and usage.
`clear()` clears the terminal display and `echo("text")` prints text; both are handled by the
terminal context.

```ts
await sys.version()
await fs.list()
await fs.touch("/notes").write("durable data")
await fs.open("/notes").read()
await fs.move("/notes", "/archive")
await fs.remove("/archive")
await service["storage"].restart()
await net.ping("10.0.2.2")
await net.fetch("http://10.0.2.2:8080/readme", "/readme")
```

Fetch response mode returns a typed response. The body is explicit and can be chained into a
bounded Storage write:

```ts
var response = net.fetch("http://10.0.2.2:8080/readme")
await response

await net.fetch("http://10.0.2.2:8080/readme").then((response) => {
    return fs.touch("/download").write(response.body)
})
```

`var` infers the value type and keeps up to eight variables for the Session lifetime. Promise
results remain in up to four fixed slots after assignment. Reassignment must preserve the inferred
type. Restarting Session or Flow clears variables and promises.

Flow source is limited to 256 bytes. It supports strings, bytes, numbers, booleans, identifiers,
members, indexes, calls, `await`, and one-argument expression/block arrow callbacks. `return` is
valid only inside a callback. Conditionals, loops, imports, dynamic objects, and `try`/`catch` are
deferred. Global `fetch(url)` is deferred; use `net.fetch(url)`.

Type failures include a bounded diagnostic and source span. The failed expression does not cancel
unrelated promise slots. Ctrl-C cancels the foreground evaluation; `promise.cancel()` cancels that
promise explicitly. Completion is owned by Flow and is generated from the same typed registry.
Late completion replies are discarded by request ID and line revision.

Paths are root-relative when written without `/`. Mutating Storage operations use one bounded
Begin → operation → Commit transaction. `net.fetch(url, destination)` stages and atomically
publishes the destination. `net.fetch(url)` transports a bounded `Response` body through Fetch;
Flow never reads Network-owned state directly.
