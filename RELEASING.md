# Releasing

icofon ships three ways: a Homebrew formula, a crates.io package, and
`cargo install` from a clone. The version in `Cargo.toml` is the single source
of truth for all three.

Nothing here is automated yet — these are the steps, run by hand.

## 1. Cut the version

```bash
# Bump `version` in Cargo.toml, then:
cargo test
cargo build --release
cargo publish --dry-run
```

`cargo publish --dry-run` catches the packaging problems that only show up in a
clean checkout: a file that is git-ignored but needed, or metadata crates.io
rejects.

Update `Cargo.lock` in the same commit — it is committed, so a release built
from the tag resolves exactly the dependencies that were tested.

```bash
git commit -am "Release v0.2.0"
git tag -a v0.2.0 -m "v0.2.0"
git push origin master --follow-tags
```

## 2. Publish to crates.io

```bash
cargo publish
```

This is irreversible: a published version can be yanked but never replaced. Run
step 1's dry run first.

## 3. Publish the Homebrew formula

The formula lives in a personal tap, so users install with
`brew install jwo1f/tap/icofon`. A tap is a plain GitHub repository named
`homebrew-tap`; create it once:

```bash
gh repo create JWo1F/homebrew-tap --public
```

Get the checksum of the release tarball GitHub generates from the tag:

```bash
curl -sL https://github.com/JWo1F/icofon/archive/refs/tags/v0.2.0.tar.gz | shasum -a 256
```

Then write `Formula/icofon.rb` in the tap, with `url`, `version` and `sha256`
pointing at the new tag:

```ruby
class Icofon < Formula
  desc "Build an icon font (TTF + CSS) from a folder of SVG files"
  homepage "https://github.com/JWo1F/icofon"
  url "https://github.com/JWo1F/icofon/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "PASTE_THE_CHECKSUM_HERE"
  license "MIT"
  head "https://github.com/JWo1F/icofon.git", branch: "master"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    (testpath/"icons").mkpath
    (testpath/"icons/check.svg").write <<~SVG
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
        <path d="M4 12l5 5L20 6" fill="none" stroke="currentColor" stroke-width="2"/>
      </svg>
    SVG

    system bin/"icofon", testpath/"icons", testpath/"out/icofon.ttf", "--no-html"

    assert_path_exists testpath/"out/icofon.ttf"
    assert_match "icon-check", (testpath/"out/icofon.css").read
  end
end
```

The test builds a real font from a stroke-drawn icon, which is the conversion
most likely to break, and checks the icon reached the stylesheet. Verify it
before pushing the tap:

```bash
brew install --build-from-source ./Formula/icofon.rb
brew test icofon
brew audit --strict --new ./Formula/icofon.rb
```

`brew audit --new` is the stricter set of checks Homebrew applies to a formula
it has not seen before; run it on the first submission.

## 4. Build the bottles

A bottle is a precompiled build. Without one, `brew install` compiles from
source and pulls in the whole Rust toolchain — several minutes and a large
download for something that takes 1.1 MB to ship.

Once the formula in the tap points at the new tag, run the `Bottle` workflow —
it lives in the tap, not here:

```bash
gh workflow run bottle.yml --repo JWo1F/homebrew-tap -f version=0.2.0
```

It builds on macOS 26, 15 and 14 (Apple Silicon) and macOS 15 (Intel), attaches
the tarballs to a release on the tap, and commits the `bottle do` block to the
formula. Homebrew falls back to a bottle from an older macOS of the same
architecture, so those four cover more than they look like they do; anything
else — Linux, older macOS — still builds from source, which works.

Both writes land in the tap, so the workflow runs on the built-in
`GITHUB_TOKEN`. There is no secret to create, rotate or leak. The bottles are
hosted on the tap's releases rather than beside the source tarball here, which
is what buys that.

**Bottles are not byte-reproducible.** Re-running the workflow for a version
that already has bottles replaces the tarballs and so changes their checksums,
which invalidates whatever is already in the formula. Let the workflow write the
block — it uploads and commits from the same build — rather than copying hashes
from an earlier run.

## Moving to homebrew-core later

Once in core, Homebrew's own CI builds the bottles for every platform they
support and the `bottle do` block is maintained for you — the tap's `Bottle`
workflow goes away with it.

A personal tap needs no approval and can be published the day a release is cut.
`homebrew-core` — where `brew install icofon` works without the tap prefix — has
an acceptance bar: a stable, versioned, reasonably used project. Once icofon
meets it, the same formula can be submitted there, and the tap kept as an alias.

## Checklist

- [ ] `Cargo.toml` version bumped, `Cargo.lock` updated
- [ ] `cargo test` passes
- [ ] `cargo publish --dry-run` clean
- [ ] Tag pushed
- [ ] `cargo publish`
- [ ] Formula `url`, `version` and `sha256` updated in the tap
- [ ] `brew install --build-from-source` and `brew test` pass
- [ ] `Bottle` workflow run, and the `bottle do` block in the formula matches
      the tarballs actually attached to the release
- [ ] `brew install` on a clean cache pours the bottle instead of compiling
