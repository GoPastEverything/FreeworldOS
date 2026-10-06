# M4-A — Native binary-safe VFS graph

Status: implementation branch; depends on the M3.5-C scheduler line only for repository sequencing, not for VFS mechanics.

M4-A replaces the VFS placeholder with the first native FreeWorld object graph.

## Scope

M4-A deliberately implements only the format-neutral native substrate:

- one native root identity;
- stable native node IDs;
- directories;
- RAM-backed files;
- owned binary-safe name segments;
- exact child lookup;
- parent relationships;
- path resolution as `root object + sequence of native segments`.

It does **not** implement mount tables, Windows/Linux path parsing, case-folding, filesystem drivers, permissions, symlinks, hard links or namespace projections yet.

## Native names

A native name owns:

~~~text
raw bytes + NameEncoding
~~~

The encoding tag remains provenance/round-trip metadata, not a promise of valid human Unicode.

For the M4-A segment constructor:

- empty segments are rejected;
- NUL is rejected;
- `Opaque` segments reject `/`, matching the Linux byte-name boundary while allowing backslash as ordinary data;
- `Wtf8` segments reject both `/` and `\\` because those are projection separators rather than stored native segment identity.

No UTF-8 normalization or lossy string conversion is performed.

## Graph identity

The graph begins with one root node. Children are referenced by `NodeId`, not path strings.

A path resolves as:

~~~text
RootHandle
   +
[Name segment, Name segment, ...]
   |
   v
native child lookup
   |
   v
NodeId
~~~

Two equal byte sequences with different encoding/provenance tags are distinct native names in M4-A.

Only `CaseSensitive` lookup is implemented in this slice. The existing case-insensitive policies remain explicit but return `UnsupportedNamePolicy` until a correct Unicode/foreign-name comparison design is frozen. M4-A does not substitute ASCII-only case folding and call it Windows compatibility.

## RAM-backed files

A file node owns an in-memory byte vector.

M4-A supports:

- create file;
- replace file contents;
- read exact file bytes.

The payload is binary. Embedded NUL and non-UTF-8 bytes are ordinary data.

## CI proof

The `m4a-ci-self-test` image constructs a local graph and proves:

- a directory name containing non-UTF-8 byte `0xff` is preserved;
- an opaque/Linux-originated segment containing backslash is treated as data, not a separator;
- exact root+segment path resolution reaches the expected node;
- RAM-file bytes round-trip exactly, including NUL and `0xff`;
- duplicate child creation is rejected;
- opaque names containing `/` are rejected;
- WTF-8 names containing a Windows separator are rejected;
- an unknown root handle cannot resolve a path;
- directory/file node kinds and parent identity are stable.

The named result is:

~~~text
FreeWorldOS: SELFTEST PASS name=m4a.vfs_graph
FreeWorldOS: M4-A VFS graph self-test: passed binary_names=ok exact_lookup=ok ram_file=ok root_identity=ok separators=projection_only
~~~

## Deferred to later M4 slices

- production/global graph synchronization;
- mount/root table;
- FreeWorld namespace projections;
- Linux and Windows path decoders;
- correct case-insensitive comparison policy;
- node removal/rename;
- directory enumeration;
- permissions and security labels;
- symlinks/hard links;
- persistent filesystem drivers.

M4-A is intentionally the native object graph underneath those features, not an imitation of either Windows or Linux path internals.
