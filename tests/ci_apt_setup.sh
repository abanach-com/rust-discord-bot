#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf -- "$test_dir"' EXIT
cd -- "$test_dir"
mkdir bin

cat > bin/sudo <<'EOF'
#!/usr/bin/env bash
if [[ "$1" == -n ]]; then
    shift
fi
exec "$@"
EOF

cat > bin/getent <<'EOF'
#!/usr/bin/env bash
exit 2
EOF

cat > bin/apt-get <<'EOF'
#!/usr/bin/env bash
case " $* " in
    *' update '*)
        echo "Temporary failure resolving 'archive.ubuntu.com'" >&2
        case " $* " in
            *' APT::Update::Error-Mode=any '*) exit 100 ;;
        esac
        # Without strict update mode, APT can fall back to stale indexes.
        exit 0
        ;;
    *' install '*) touch install-attempted ;;
    *) exit 2 ;;
esac
EOF

chmod +x bin/sudo bin/getent bin/apt-get
status=0
PATH="$test_dir/bin:$PATH" ImageOS=ubuntu24 ImageVersion=test-image \
    timeout --kill-after=1s 10s bash "$repo_root/scripts/install-ci-dependencies.sh" > setup.out 2>&1 || status=$?

if [[ "$status" != 100 || -e install-attempted ]]; then
    cat setup.out >&2
    echo "Expected APT exit 100 with no install attempt after incomplete refresh, got $status." >&2
    exit 1
fi

if ! grep -q -F 'test-image' apt-install.log ||
   ! grep -q -F 'DNS lookup failed for' apt-install.log ||
   ! grep -q -F "Temporary failure resolving 'archive.ubuntu.com'" apt-install.log; then
    cat setup.out >&2
    echo 'Expected runner, DNS and APT diagnostics in apt-install.log.' >&2
    exit 1
fi

echo 'CI APT setup preserves the refresh failure and diagnostics.'
