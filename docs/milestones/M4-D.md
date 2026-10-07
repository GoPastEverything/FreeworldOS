# M4-D — Linux absolute byte-path projection

Status: stacked implementation branch on corrected M4-C.

M4-D adds the first compatibility path-string decoder on top of the native VFS/root model.

It intentionally implements Linux absolute byte paths only. Windows UTF-16/WTF-8 parsing remains a separate later slice.

## Input contract

The decoder accepts a raw byte slice representing an absolute Linux path.

Examples:

~~~text
/
///usr//bin/tool
/state\bin
~~~

Linux names remain raw bytes. No UTF-8 conversion occurs.

The parser:

- requires the first byte to be '/';
- rejects NUL anywhere in the input;
- splits only on '/';
- collapses repeated '/' separators;
- preserves '\\' as ordinary filename data;
- produces OwnedName values with NameEncoding::Opaque;
- binds the resulting path to ProjectionRoot::LinuxRoot.

## Traversal status

M4-D deliberately rejects '.' and '..' segments with UnsupportedTraversal.

This is not because Linux lacks those semantics. It is because native parent traversal, root confinement and future per-process namespace policy must be defined together.

M4-D refuses unsupported traversal instead of implementing a string-only approximation that could later violate namespace confinement.

## Owned projection path

The decoded form is:

~~~text
ProjectionPath {
    root: ProjectionRoot::LinuxRoot,
    segments: Vec<OwnedName>,
}
~~~

The decoded object owns its segment bytes, so it does not borrow the caller's path buffer.

## Production resolution

M4-D adds production resolution for an Owned ProjectionPath:

~~~text
ProjectionRoot
   |
   v
RootTable::handle_for()
   |
   v
RootHandle -> native start NodeId
   |
   v
owned native name segments
   |
   v
exact VfsGraph child lookup
   |
   v
target NodeId
~~~

No Linux string becomes native object identity.

## M4-D proof

The M4-B/M4-C scheduled reader already has a LinuxRoot binding whose native directory contains an opaque file named:

~~~text
b"state\\bin"
~~~

M4-D parses the raw Linux path:

~~~text
b"/state\\bin"
~~~

Since Linux uses '/' as the separator, the backslash remains part of the single filename segment.

The parsed path must resolve to the exact same native file NodeId already proven by M4-B and M4-C.

Additional checks prove:

- "state\\bin" without leading '/' -> NotAbsolute;
- "/./state" -> UnsupportedTraversal;
- an embedded NUL -> Nul;
- "///" collapses to a root-only path and resolves to the bound Linux root node.

Expected marker:

~~~text
FreeWorldOS: M4-D Linux path proof: input=/state\\bin file_node=... raw_bytes=ok backslash=data repeated_slash=collapsed dot_traversal=refused
~~~

## Deferred

M4-D does not yet implement:

- relative Linux paths;
- current-working-directory state;
- '.' / '..' traversal;
- symlink traversal;
- mount crossing during traversal;
- permissions;
- Windows DOS path parsing;
- Windows NT path parsing;
- UTF-16/WTF-8 conversion;
- case-insensitive lookup.

The next Windows path slice must preserve unpaired UTF-16 surrogates losslessly and must not pretend ordinary UTF-8 is sufficient.
