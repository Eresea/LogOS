# Wish List – Modern Object & Namespace Model

> These are exploratory ideas for future versions of LogOS. They are not part of Core v1 and should be evaluated individually before implementation.

---

## Guiding Principle

Traditional operating systems expose a filesystem as the primary abstraction.

LogOS should instead expose a unified object model, where files, services, devices, applications and AI capabilities are all first-class objects.

The filesystem remains an important user interface, but is no longer the underlying model.

---

# Objects First

Every persistent or live entity should be represented as an object.

Potential properties:

- Stable identifier
- Type
- Metadata
- Owner
- Capabilities
- Relationships
- Version history
- Optional path aliases

Objects survive moves and renames.

A path should never be the object's identity.

---

# Multiple Views

The same object may appear through multiple navigation views.

Examples:

- Path view
- Recent items
- Search results
- Workspace view
- Project view
- Application view

Moving an object between views should not duplicate it.

---

# Paths Become Views

Keep a traditional tree because it is familiar and scriptable.

However, the tree is only one view over the object graph.

Example:

```
/workspaces/logos/docs/roadmap.md
```

may reference the exact same object shown in:

```
Recent
Modified Today
Workspace "LogOS"
```

---

# Semantic Root Namespaces

Instead of inheriting Unix layout, use semantic namespaces.

Example:

```
/system
/apps
/users
/workspaces
/data
/devices
/runtime
```

These represent concepts rather than historical implementation details.

---

# Users Are Not Security Boundaries

Unlike Unix or Windows, `/users/<user>` should primarily represent a personal namespace.

Permissions are determined by:

- Identity
- Capabilities
- Service policy
- Object ownership

not by directory location.

Moving an object should not change its permissions.

Guiding principle:

> Paths organize objects. Capabilities authorize actions.

---

# Workspaces as First-Class Objects

Projects should become first-class concepts rather than merely directories.

Example:

```
/workspaces/logos
```

A workspace may contain:

- Documents
- Source code
- Build configuration
- AI context
- Git metadata
- Relationships
- Shared permissions

without requiring these to be physically colocated.

---

# Immutable Applications

Applications should be installed as immutable packages.

Separate:

- Application binaries
- User configuration
- User state
- Cache
- Shared data

Applications should never scatter hidden files throughout the system.

Potential future features:

- Atomic installation
- Rollback
- Multiple installed versions
- Signed packages
- Garbage collection

---

# Typed Configuration

Configuration should be represented as typed objects rather than arbitrary text files.

Potential commands:

```
config get terminal.font
config set network.hostname
config rollback network
```

Text representations can still exist as one view if desired.

---

# Service Namespaces

Introduce service references independent of the filesystem.

Examples:

```
@browser
@terminal
@calendar
@mail
@clipboard
@camera
@image
@filesystem
```

These reference live capabilities instead of files.

---

# Uniform Object Addressing

The same syntax could address many resource types.

Examples:

```
@workspace/logos
@mail/inbox
@browser/history
@camera/front
@image
```

Some references are persistent.

Others represent live services.

The addressing model remains consistent.

---

# Service Pipelines

Service references integrate naturally with Flow pipelines.

Example:

```
report.md
    |> @translate fr
    |> @pdf export
    |> @mail send alice
```

Services become composable processing stages rather than standalone executables.

---

# Introspection

Every object should support inspection.

Examples:

```
inspect @browser
inspect @workspace/logos
inspect @camera
inspect roadmap.md
```

Inspection may expose:

- Metadata
- Current state
- Available commands
- Relationships
- Permissions
- Statistics

This creates a uniform experience across files, services, devices and applications.

---

# AI-Native Navigation

Navigation should not depend solely on remembering paths.

Users should naturally move between:

- Search
- Recent
- Relationships
- Workspaces
- Semantic namespaces
- AI-assisted discovery

Paths remain available, but are no longer the primary navigation model.

---

# Overall Vision

Rather than replacing the filesystem, LogOS should demote it.

The operating system is fundamentally an object graph with typed services.

The filesystem is one navigation interface among many, preserving compatibility with scripts and developer workflows while allowing richer, AI-native interactions.