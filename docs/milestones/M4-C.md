# M4-C — Root bindings and compatibility projections

Status: stacked implementation branch on M4-B.

M4-C adds the first explicit namespace-root binding layer over the one persistent native VFS graph.

## One graph, multiple roots

M4-C does not create a Windows filesystem and a Linux filesystem.

A root binding is:

~~~text
RootHandle
  ->
native NodeId
  +
ProjectionRoot metadata
~~~

Multiple root handles may explicitly reference the same native NodeId.

That implements the FreeWorld architecture rule:

~~~text
one native object, multiple compatibility projections
~~~

without duplicating file objects or file bytes.

## Root projection kinds

M4-C defines:

- Native;
- LinuxRoot;
- WindowsDrive(letter).

The root metadata identifies presentation/compatibility intent. It does not alter the underlying object identity.

Windows drive letters must currently be uppercase ASCII A-Z. M4-C rejects lowercase/non-drive selectors rather than silently normalizing them.

Only one binding for a given projection selector is allowed. A second attempt to bind Windows C:, for example, returns DuplicateProjection.

## Bounded root table

The bootstrap root table is fixed-capacity:

~~~text
ROOT_BINDING_CAPACITY = 16
~~~

The native root occupies the first entry. Additional projection roots receive independent RootHandle values beginning at 0x1000.

Root lookup performs no heap allocation.

This capacity is an implementation bound for the bootstrap milestone, not a public ABI promise.

## Resolution

M4-A path resolution began only at the graph's native root.

M4-C adds:

~~~text
resolve_from(native_start_node, segments)
~~~

Production resolution now performs:

~~~text
Path.root
   |
   v
RootTable
   |
   v
native start NodeId
   +
Path.segments
   |
   v
VfsGraph::resolve_from()
   |
   v
native target NodeId
~~~

The RootHandle is therefore presentation/namespace identity. The final resolved object remains a native NodeId.

## Explicit visibility

Cross-facet visibility is never automatic.

A native node becomes visible through a Linux or Windows root only when code explicitly installs that binding.

Binding the same node twice under different projections is allowed:

~~~text
native volume NodeId 42
   |                    |
   v                    v
LinuxRoot handle     WindowsDrive(C:) handle
   |                    |
   +----------+---------+
              |
              v
          NodeId 42
~~~

The projection bindings are aliases, not copies.

## M4-C proof

M4-C extends the existing M4-B scheduled writer/reader proof.

The writer creates one native directory containing the shared file, then explicitly binds that directory as:

- LinuxRoot;
- WindowsDrive('C').

The writer proves:

- Linux and Windows handles are distinct;
- both projection lookups return the installed handles;
- a duplicate Windows C: binding is rejected.

The reader then resolves the same file twice:

~~~text
Linux root handle + [file segment]
Windows C: handle + [file segment]
~~~

Both resolutions must produce the exact same native file NodeId already reached through the native M4-B path.

Expected markers:

~~~text
FreeWorldOS: M4-C roots: native_node=... linux_root=... windows_c_root=... aliases=explicit
FreeWorldOS: M4-C projection proof: linux_root=... windows_c_root=... file_node=... same_native_object=ok explicit_visibility=ok
~~~

## What M4-C does not claim

M4-C does not yet parse:

- /foo/bar;
- C:\\foo\\bar;
- NT object-manager path syntax;
- native tagged textual paths.

It also does not implement Windows case-insensitive comparison.

A WindowsDrive root binding means "this native root is explicitly exposed as this drive selector." It does not change the directory's NamePolicy.

Path string decoders and correct projection-specific name comparison remain later M4 work.

## Deferred

- Linux path decoder;
- Windows DOS/NT path decoder;
- per-process namespace projection objects;
- reversible escaping for foreign names that cannot be spelled directly;
- correct case-insensitive/case-preserving comparison;
- mount removal/rebinding;
- persistent filesystem-driver mounts;
- SMP root-table synchronization.
