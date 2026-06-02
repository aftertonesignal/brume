---
name: brume-schema-version-bump
description: >
  When and how to bump the schema version for Brume's persisted
  formats (Patch, Perf, ControlMatrix). Defines the forward-compat
  policy, the migration arm pattern, and the test surface that pins
  each gate. Triggers on "schema version", "Patch::version",
  "Perf::version", "ControlMatrix::version", "SCHEMA_VERSION",
  "bump version", "patch format change", "control matrix format",
  "migration", "patch migration", "patch-store schema",
  "control-matrix schema", "version gate".
---

# Schema-version policy and migrations

Brume persists three on-disk formats, each gated by a `version: u32`
field that the loader checks before deserializing. The policy is
"refuse the future, migrate the past, never silently lose fields."
This skill is the procedure for making any change to those formats
safely.

## The three formats

| Format          | Crate            | Path                              | Constant                            |
|-----------------|------------------|-----------------------------------|-------------------------------------|
| `Patch`         | `patch-store`    | `~/.brume/library/<mode>/<n>.json`| `SCHEMA_VERSION` (private)          |
| `Perf`          | `patch-store`    | `~/.brume/library/perf/<n>.json`  | `SCHEMA_VERSION` (shared with Patch)|
| `ControlMatrix` | `control-model`  | `~/.brume/cc-bindings.json`       | `CONTROL_MATRIX_SCHEMA_VERSION`     |

`Transport` (`~/.brume/transport.json`) is small enough that it
hasn't been schema-versioned. If you add a non-trivial field, add
versioning following the same pattern.

## The forward-compat policy

Each loader checks `parsed.version` against the build's
`SCHEMA_VERSION` and behaves as follows:

- `parsed.version == SCHEMA_VERSION` → accept.
- `parsed.version < SCHEMA_VERSION` → run the migration arm for
  the appropriate version step, then accept. (Today there are no
  migration arms because no version step has been introduced; the
  scaffolding is in place for when one is.)
- `parsed.version > SCHEMA_VERSION` → reject with
  `UnsupportedVersion`. The user gets a clear message naming the
  found version, the supported version, and the resolution
  ("update brume to read this file").

The policy is asymmetric on purpose. Older files load (eventually,
through migrations); newer files don't. Re-saving a partially-
parsed v2-as-v1 file would silently drop v2-only fields, so we
refuse to do it.

## When you need to bump `SCHEMA_VERSION`

Bump when **any of these are true**:

- A field's meaning changes (a value range, a unit, a semantic).
- A field is removed.
- A non-default field is added that older readers can't reasonably
  ignore.

You **don't** need to bump when:

- A new optional field is added with `#[serde(default,
  skip_serializing_if = "Option::is_none")]`. Older readers see
  the absent field; they ignore it.
- A new optional field is added with a `#[serde(default = "...")]`
  function. Older files deserialize with the default value; new
  files round-trip cleanly.
- A field is renamed *and* aliased with `#[serde(alias = "old_name")]`
  for one version cycle.

Patch + Perf already share `SCHEMA_VERSION`. If the change applies
to only one of them but introduces a meaningful structural shift
on that side, split them: rename to e.g. `PATCH_SCHEMA_VERSION` and
`PERF_SCHEMA_VERSION`, document the divergence at the constant,
and bump independently going forward.

## How to add a v2

Worked example: adding a v2 of `Patch` that adds a required
`signature` field.

### 1. Define the new constant in `patch-store/src/lib.rs`

```rust
const SCHEMA_VERSION: u32 = 2;
```

### 2. Update the `Patch` struct

Add the new required field as if it had always been there. Make it
serde-friendly with a sensible default for old files:

```rust
#[serde(default = "default_signature")]
pub signature: Vec<u8>,

fn default_signature() -> Vec<u8> {
    Vec::new()
}
```

If the field is required and there's no sensible default for v1
files, the migration arm fills it (next step).

### 3. Add the migration dispatch in `load`

```rust
pub fn load(&self, mode: &str, name: &str) -> Result<Patch, PatchError> {
    let path = self.patch_path(mode, name)?;
    let json = std::fs::read_to_string(&path).map_err(...)?;
    let mut patch: Patch = serde_json::from_str(&json).map_err(...)?;

    if patch.version > SCHEMA_VERSION {
        return Err(PatchError::UnsupportedVersion { ... });
    }

    // Migration ladder: each arm migrates from `v` to `v+1`. Apply
    // ladder steps until we reach SCHEMA_VERSION. The match-on-version
    // shape lets you compose multiple migrations as the format
    // evolves further.
    while patch.version < SCHEMA_VERSION {
        match patch.version {
            1 => migrate_v1_to_v2(&mut patch),
            other => return Err(PatchError::UnsupportedVersion {
                path: path.display().to_string(),
                found: other,
                supported: SCHEMA_VERSION,
            }),
        }
    }
    Ok(patch)
}

fn migrate_v1_to_v2(patch: &mut Patch) {
    // Fill the new `signature` field with whatever's correct for
    // an old file. For a real signature this might involve
    // computing it from the rest of the patch, or marking the
    // file as "unsigned, predates signing."
    patch.signature = compute_or_default_signature(patch);
    patch.version = 2;
}
```

The `while` shape means a v0 file (if such a thing existed) would
walk the ladder v0 → v1 → v2. Each migration is a small, focused
function with a name that documents what it does.

### 4. Update the save path

Save always writes the current `SCHEMA_VERSION`. If the migration
runs as part of load, the next save round-trips the file at the
new version. No special save logic needed.

`Patch::new` already sets `version: SCHEMA_VERSION`; that
constant just incremented, so new patches get v2 automatically.

### 5. Test the migration

Add a test in `patch-store/src/lib.rs` mod tests that writes a
hand-crafted v1 file, loads it, and asserts the migrated Patch
has the right v2 shape:

```rust
#[test]
fn load_migrates_v1_patch_to_v2() {
    let dir = std::env::temp_dir().join("brume-test-migrate-v1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("fm")).unwrap();
    std::fs::write(
        dir.join("fm").join("legacy.json"),
        r#"{
          "version": 1,
          "name": "legacy",
          "mode": "fm",
          "params": {}
        }"#,
    )
    .unwrap();
    let lib = PatchLibrary::open(&dir).unwrap();
    let loaded = lib.load("fm", "legacy").unwrap();
    assert_eq!(loaded.version, SCHEMA_VERSION);
    // Assert the migration filled the new field correctly:
    assert_eq!(loaded.signature, expected_default_for_legacy());
    let _ = std::fs::remove_dir_all(&dir);
}
```

Pair this with the existing `load_rejects_future_schema_version`
test which already pins the upper-bound gate.

## The `ControlMatrix` flavor

`ControlMatrix` follows the same shape but the gate lives in
`from_json` (not in `PatchLibrary::load` — `ControlMatrix` doesn't
have a library wrapper). The relevant tests are
`from_json_rejects_future_version`,
`from_json_treats_missing_version_as_current`, and
`round_trip_includes_version` in
`crates/control-model/src/lib.rs`'s `schema_tests` module. Add a
matching `from_json_migrates_v1_to_v2` test when you bump.

The `default_schema_version` serde-default function on
`ControlMatrix` ensures pre-version-field files (today's released
format) deserialize as v1. Don't remove or alias-rename it without
understanding that existing user files depend on it.

## Anti-patterns

- **Don't** bump `SCHEMA_VERSION` when you've only added an
  optional field. Adding `#[serde(default)]` to a new field is
  enough; old files keep loading without the migration arm.
- **Don't** silently widen the policy ("oh, v3 is just fine, the
  user must have a future build"). The whole point of the gate is
  to refuse data shapes this build can't reason about. If you
  need cross-version compatibility, add the migration arm.
- **Don't** modify a v1 file's interpretation in place ("v1 used
  to mean X, now we treat it as Y"). That's a silent semantic
  rewrite. Bump to v2, migrate v1 explicitly.
- **Don't** share migration helpers across formats unless the
  formats genuinely share the structural transition.
  `migrate_patch_v1_to_v2` and `migrate_perf_v1_to_v2` are
  separate functions even if they look similar.
- **Don't** ship a v2 build to users with an unfinished migration
  ladder. The combination of a v2 binary and a v1 file on disk
  with a missing migration arm produces a confusing
  `UnsupportedVersion` error pointing the wrong direction.

## When the format change is too large for a migration ladder

If v1 → v2 is fundamentally incompatible (different storage
strategy, restructured field tree), consider:

1. Cutting a clean break: bump the file extension or directory
   name (`~/.brume/library_v2/` alongside `~/.brume/library/`).
   v1 stays readable for export; v2 starts empty.
2. A one-shot bulk migrator in `brumectl` that reads v1 files,
   transforms them, writes v2 files, and the user opts in.
3. Deferring the change until you can pair it with a major
   version bump and a CHANGELOG entry titled "BREAKING."

The migration ladder shape is good for incremental field
additions and renames, not for top-down rewrites.
