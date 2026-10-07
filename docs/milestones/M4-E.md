# M4-E — Windows DOS absolute UTF-16 path projection

Status: stacked implementation branch on M4-D.

M4-E adds the first Windows path decoder on top of the native FreeWorld VFS projection model.

It implements DOS drive-absolute paths only. NT object-manager paths, UNC paths and Win32 normalization remain later work.

## Input contract

The parser accepts UTF-16 code units representing a drive-absolute DOS path:

~~~text
C:\\foo\\bar
C:/foo/bar
~~~

The parser:

- requires a drive letter followed by ':' and a separator;
- accepts '\\' and '/' as separators;
- normalizes lowercase drive letters to uppercase;
- rejects NUL;
- collapses repeated separators;
- rejects '.' and '..' with UnsupportedTraversal;
- rejects ':' inside later path segments in this slice;
- converts each UTF-16 segment into lossless WTF-8;
- returns ProjectionRoot::WindowsDrive(uppercased_letter).

Drive-relative syntax such as:

~~~text
C:foo
~~~

is explicitly rejected as NotDriveAbsolute.

## WTF-8 requirement

Windows filenames are UTF-16 and may contain unpaired surrogate code units.

M4-E does not replace those units with U+FFFD and does not pretend they are normal UTF-8.

The decoder performs:

- valid surrogate pair -> ordinary four-byte UTF-8 scalar encoding;
- unpaired high surrogate -> direct three-byte WTF-8 encoding;
- unpaired low surrogate -> direct three-byte WTF-8 encoding;
- ordinary BMP code unit -> ordinary UTF-8 encoding.

The resulting native segment is tagged NameEncoding::Wtf8.

## Native identity remains separate

A parsed Windows path becomes:

~~~text
ProjectionPath {
    root: ProjectionRoot::WindowsDrive(letter),
    segments: Vec<OwnedName{ encoding=Wtf8 }>
}
~~~

The drive selector chooses an explicitly installed root binding. The string C:\\... is not stored as native object identity.

## M4-E proof

The existing M4-B writer creates the shared native directory used by M4-C.

Under M4-E it additionally creates one Windows-origin file whose native name is the WTF-8 encoding of:

~~~text
UTF-16:
w i n - D800 . d a t
          ^
          unpaired high surrogate
~~~

Native WTF-8 bytes:

~~~text
77 69 6e 2d ed a0 80 2e 64 61 74
~~~

The file receives its own binary payload and native NodeId.

The reader then parses this UTF-16 DOS path:

~~~text
C:\\win-<unpaired D800>.dat
~~~

The parser must emit WindowsDrive('C') plus the exact WTF-8 segment. Production root resolution must then reach the exact native NodeId created by the writer.

The proof also checks:

- lowercase c:\\ normalizes to WindowsDrive('C');
- C:foo is rejected as drive-relative;
- C:\\.\\ is rejected until dot traversal semantics exist.

Expected markers:

~~~text
FreeWorldOS: M4-E writer: windows_file=... wtf8_unpaired_surrogate=stored bytes=11
FreeWorldOS: M4-E Windows path proof: drive=C file_node=... utf16=ok wtf8_unpaired_surrogate=ok lowercase_drive=normalized dot_traversal=refused
~~~

## What M4-E does not claim

M4-E does not yet implement:

- UNC paths;
- \\?\ extended-length paths;
- \\.\ device paths;
- NT object-manager paths;
- Win32 reserved-name rules;
- alternate data streams;
- current-drive/current-directory semantics;
- '.' or '..' traversal;
- case-insensitive lookup;
- case-preserving comparison;
- symlink/junction/reparse behavior.

Those are separate observable compatibility behaviors and must be implemented deliberately rather than hidden inside string normalization.
