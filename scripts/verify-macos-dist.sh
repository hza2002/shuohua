#!/bin/bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 <archive.tar.gz> [cargo-target-dir]" >&2
    exit 2
fi

archive="$1"
build_root="${2:-}"
checksum="$archive.sha256"
archive_name="$(basename "$archive")"
package_name="${archive_name%.tar.gz}"

[[ -f "$archive" ]] || { echo "ERROR: missing archive: $archive" >&2; exit 1; }
[[ -f "$checksum" ]] || { echo "ERROR: missing checksum: $checksum" >&2; exit 1; }

checksum_entries="$(awk 'END { print NR }' "$checksum")"
read -r expected_hash expected_name < "$checksum"
if [[ "$checksum_entries" -ne 1 || "${#expected_hash}" -ne 64 \
    || "$expected_hash" == *[!0-9a-f]* || "$expected_name" != "$archive_name" ]]; then
    echo "ERROR: checksum must contain exactly one SHA-256 entry for $archive_name" >&2
    exit 1
fi
actual_hash="$(shasum -a 256 "$archive" | awk '{ print $1 }')"
[[ "$actual_hash" == "$expected_hash" ]] || {
    echo "ERROR: checksum mismatch for $archive_name" >&2
    exit 1
}
echo "$archive_name: OK"

expected_entries="$(printf '%s\n' \
    "$package_name/" \
    "$package_name/LICENSE" \
    "$package_name/README.en.md" \
    "$package_name/README.md" \
    "$package_name/shuo" | sort)"
actual_entries="$(tar -tzf "$archive" | sort)"
if [[ "$actual_entries" != "$expected_entries" ]]; then
    echo "ERROR: archive contents do not match the release contract" >&2
    diff -u <(printf '%s\n' "$expected_entries") <(printf '%s\n' "$actual_entries") || true
    exit 1
fi

extract_root="$(mktemp -d "${TMPDIR:-/tmp}/shuo-dist-verify.XXXXXX")"
trap 'rm -rf "$extract_root"' EXIT
tar -xzf "$archive" -C "$extract_root"

for file_name in LICENSE README.md README.en.md; do
    file="$extract_root/$package_name/$file_name"
    [[ -f "$file" && ! -L "$file" ]] || {
        echo "ERROR: $file_name is not a regular file" >&2
        exit 1
    }
done

verify_macho() {
    local binary="$1"
    local label="$2"
    local expected_bundle_id="$3"
    local archs minos sdk dependency

    [[ -f "$binary" && ! -L "$binary" ]] || {
        echo "ERROR: $label is not a regular file" >&2
        exit 1
    }

    archs="$(lipo -archs "$binary")"
    [[ "$archs" == "arm64" ]] || {
        echo "ERROR: $label architecture is '$archs', expected 'arm64'" >&2
        exit 1
    }

    minos="$(vtool -show-build "$binary" | awk '$1 == "minos" { print $2; exit }')"
    [[ "$minos" == "15.0" ]] || {
        echo "ERROR: $label minos is '$minos', expected '15.0'" >&2
        exit 1
    }

    sdk="$(vtool -show-build "$binary" | awk '$1 == "sdk" { print $2; exit }')"
    [[ -n "$sdk" && "${sdk%%.*}" -ge 26 ]] || {
        echo "ERROR: $label SDK is '$sdk', expected 26 or newer" >&2
        exit 1
    }

    while IFS= read -r dependency; do
        case "$dependency" in
            /usr/lib/*|/System/Library/*) ;;
            *)
                echo "ERROR: $label has non-system dependency: $dependency" >&2
                exit 1
                ;;
        esac
    done < <(otool -L "$binary" | tail -n +2 | awk '{ print $1 }')

    if [[ -n "$expected_bundle_id" ]]; then
        local plist bundle_id purpose
        plist="$extract_root/$label.Info.plist"
        otool -X -s __TEXT __info_plist "$binary" 2>/dev/null \
            | awk '{
                for (i = 2; i <= NF; i++) {
                    word = $i
                    for (j = 7; j >= 1; j -= 2) printf "%s", substr(word, j, 2)
                }
            } END { print "" }' \
            | xxd -r -p > "$plist"
        bundle_id="$(plutil -extract CFBundleIdentifier raw -o - "$plist" 2>/dev/null)" || {
            echo "ERROR: $label has no embedded CFBundleIdentifier" >&2
            exit 1
        }
        purpose="$(plutil -extract NSMicrophoneUsageDescription raw -o - "$plist" 2>/dev/null)" || {
            echo "ERROR: $label has no embedded NSMicrophoneUsageDescription" >&2
            exit 1
        }
        [[ "$bundle_id" == "$expected_bundle_id" && -n "$purpose" ]] || {
            echo "ERROR: $label has an invalid embedded microphone usage declaration" >&2
            exit 1
        }
    fi
}

binary="$extract_root/$package_name/shuo"
verify_macho "$binary" "shuo" "com.hza2002.shuohua"
"$binary" --version

if [[ -n "$build_root" ]]; then
    for helper_name in apple_helper apple_capture_helper; do
        helpers="$(find "$build_root/aarch64-apple-darwin/release/build" \
            -path "*/out/$helper_name" -type f)"
        helper_count="$(printf '%s\n' "$helpers" | sed '/^$/d' | wc -l | tr -d ' ')"
        [[ "$helper_count" -eq 1 ]] || {
            echo "ERROR: expected one $helper_name build output, found $helper_count" >&2
            exit 1
        }
        expected_bundle_id=""
        if [[ "$helper_name" == "apple_capture_helper" ]]; then
            expected_bundle_id="com.hza2002.shuohua.apple-capture-helper"
        fi
        verify_macho "$helpers" "$helper_name" "$expected_bundle_id"
    done
fi

echo "verified portable macOS artifact: $archive"
