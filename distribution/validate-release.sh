#!/bin/sh
# Validate one release archive and its staging directory.
# Usage: validate-release.sh TARGET ARCHIVE.tar.gz STAGING_DIR
set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: $0 TARGET ARCHIVE.tar.gz STAGING_DIR" >&2
    exit 2
fi

target=$1
archive=$2
staging_dir=$3
archive_stem="work-$target"

case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu)
        ;;
    *)
        echo "error: unsupported release target: $target" >&2
        exit 1
        ;;
esac

[ "$(basename "$archive")" = "$archive_stem.tar.gz" ] || {
    echo "error: archive name does not match target: $archive" >&2
    exit 1
}
[ -x "$staging_dir/work" ] || {
    echo "error: staged work binary is missing or not executable" >&2
    exit 1
}

for metadata in README.md LICENSE; do
    [ -f "$staging_dir/$metadata" ] || {
        echo "error: staged release metadata is missing: $metadata" >&2
        exit 1
    }
done

[ -f "$archive" ] || {
    echo "error: release archive is missing: $archive" >&2
    exit 1
}

archive_files=$(tar -tzf "$archive" | sed 's#^\./##')
for required in work README.md LICENSE; do
    printf '%s\n' "$archive_files" | grep -Fqx "$required" || {
        echo "error: release archive is missing: $required" >&2
        exit 1
    }
done

echo "validated $archive_stem ($target)"
