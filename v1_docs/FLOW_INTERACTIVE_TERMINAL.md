## Interactive Flow, Addressing, and Conversation Contexts

The normal LogOS terminal should act as an interactive Flow environment
rather than maintain a separate traditional shell language. The terminal
remains a presentation and interaction surface; Flow evaluates typed
expressions and invokes LogOS system APIs.

### Interactive system API

System functionality should be exposed as typed namespaces, registries,
resources, properties, and operations.

``` flow
sys.version
net.status
net.ping("10.0.2.2")

service["storage"].status
service["storage"].restart()
```

Rules:

-   `.` is the normal member-access operator.
-   `[]` selects an entry from a registry or indexed resource.
-   Properties represent state or data and omit `()`.
-   Methods represent operations or explicit queries and use `()`.
-   Strings remain explicitly quoted.
-   Avoid exposing lookup mechanics such as
    `service.list.find("storage")`; expose the conceptual resource
    directly.
-   Interactive expressions should remain valid, composable Flow rather
    than use a separate command grammar.
-   `net`, `fs`, `service`, `sys`, and similar roots should be system
    bindings exposed to Flow rather than special language keywords.

The language should be optimized for clarity, typing, and composition.
The terminal editor should be optimized for typing speed.

### Contextual completion

Completion should behave more like a modern code editor than a
traditional shell token completer.

Typing `sys.` should display a small contextual list such as:

``` text
version       OS version
uname         OS identity
shutdown()    Shut down the system
reboot()      Restart the system
```

The completion UI should show several nearby candidates, allow `↑` / `↓`
traversal, use `Tab` to accept, provide concise descriptions and
signatures, complete surrounding syntax where appropriate, and use live
system information for registry values such as service names.

Eventually, member completion should derive from Flow types rather than
hard-coded command categories.

This makes explicit syntax such as `service["storage"].restart()`
inexpensive to type without weakening the language.

### Help and discovery

Autocomplete is the primary answer to **"what can I use here?"**.

`help` remains for deeper documentation:

``` flow
help()
help(sys)
help(sys.reboot)
help(service["storage"])
```

Long term, `help` should preferably operate on Flow symbols and values
rather than require string-based lookup.

-   Completion answers **what can I type here?**
-   `help(x)` answers **what is this and how do I use it?**
-   `help()` provides general discovery and language/system guidance.

### Transient human interfaces

The terminal may expose transient TUIs for operations that are more
efficient as direct interaction.

For filesystem traversal, holding `Alt` may temporarily render the
current location. While held:

``` text
↑ / ↓    select
→        enter directory / inspect or preview file
←        parent
```

Releasing `Alt` exits the transient view while preserving the newly
selected filesystem location.

Flow remains available when explicit or programmable filesystem
operations are required.

> Do not require users to issue commands merely to view or navigate
> state the terminal already knows how to present.

### Addressed natural-language messages

`@` is the general addressing syntax for sending natural-language input
to another entity.

``` text
@Logos Why did the previous command fail?
@BuildAgent Review this result.
@Alice Are you free tomorrow?
```

`@` is not AI-specific. A target may be the LogOS assistant, another AI
agent, a human contact, or another future message-capable entity.

Targets should participate in intelligent completion. For human
contacts, LogOS may resolve an appropriate configured communication
method. Explicit programmable messaging remains available through Flow
APIs where deterministic routing or automation is required.

### Conversation contexts

`@@target` attaches the terminal to an ongoing conversation with a
target:

``` text
@@Logos
```

After entering the conversation context, normal entered text is treated
as conversation messages and no longer requires the `@Logos` prefix.

``` text
@@ Logos

> Why is the network service failing?

...

> Can you inspect the previous ping as well?
```

The terminal should provide a fast keyboard action to leave the
conversation context and return to normal Flow input.

**Leaving a conversation does not close it.**

A conversation is a persistent resource independent of the terminal
view. Agents may continue working and human contacts may reply while the
user is elsewhere in the terminal.

Re-entering `@@Logos` should resume the current/default conversation
with Logos where it was left.

Initially, one current/default conversation per target is sufficient.
Multiple named or selectable conversations per target may be introduced
later without changing the basic `@@target` interaction.

### Background conversations

Conversations must not block normal terminal use.

``` text
@@Logos
... conversation ...

<leave>

net.status
service["network"].status

@@Logos
... resumed conversation ...
```

Background activity should be represented through unobtrusive terminal
UI rather than injecting asynchronous messages into normal Flow output.

For example:

``` text
Alice       1 new message
Logos       generating...
BuildAgent  running
```

Typing `@@` should provide completion over recent/open conversations and
available targets.

Explicitly closing a conversation is separate from leaving it and should
ultimately be exposed through the conversation/chat API.

### Context-aware AI

AI interactions should be able to receive explicitly bounded terminal
context when appropriate, such as the previous Flow expression, its
typed result or diagnostic, relevant current terminal/session context,
selected system state the user permits, and referenced resources.

This enables:

``` text
> net.ping("10.0.2.2")
Timeout

> @Logos Why?
```

without requiring the user to reproduce information already present in
the session.

Context access must remain capability-aware and inspectable.

### Interaction model

The resulting terminal has several complementary interaction forms:

``` text
Flow
    net.ping("10.0.2.2")
    service["storage"].status

Transient TUI
    Alt + arrows

One-shot addressing
    @Logos Why did this fail?

Conversation context
    @@Logos
```

These share the same terminal surface without collapsing into the same
abstraction.

The terminal is not merely a text command runner. It is an interactive
system surface hosting Flow evaluation, structured system interaction,
transient TUIs, intelligent completion, and persistent conversations.
