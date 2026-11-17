#!/usr/bin/env bash
set -euo pipefail
shopt -s nullglob

run_apt() {
    sudo -n timeout --verbose --kill-after=5s 120s apt-get \
        -o Acquire::Retries=1 \
        -o Acquire::http::Timeout=10 \
        -o Acquire::https::Timeout=10 \
        "$@"
}

{
    printf 'Runner image: %s (%s)\n' "${ImageOS:-unknown}" "${ImageVersion:-unknown}"
    sed -n 's/^PRETTY_NAME=/OS: /p' /etc/os-release

    apt_sources=()
    for source in /etc/apt/sources.list /etc/apt/sources.list.d/*.list \
        /etc/apt/sources.list.d/*.sources /etc/apt/apt-mirrors.txt; do
        if [[ -r "$source" ]]; then
            apt_sources+=("$source")
        fi
    done
    printf 'APT source and mirror files:\n'
    printf '  %s\n' "${apt_sources[@]}"

    # Extract only hostnames, without logging URL credentials or paths.
    hosts="$(awk '
        {
            sub(/(^|[[:space:]])#.*/, "")
            for (i = 1; i <= NF; i++) {
                if ($i !~ /^(mirror[+])?https?:\/\//) continue
                host = $i
                sub(/^[^:]+:\/\//, "", host)
                sub(/[\/?#].*$/, "", host)
                sub(/^.*@/, "", host)
                sub(/:[0-9]+$/, "", host)
                if (host ~ /^[[:alnum:]][[:alnum:].-]*$/) print host
            }
        }
    ' "${apt_sources[@]}" </dev/null | sort -u)"
    while IFS= read -r host; do
        [[ -n "$host" ]] || continue
        if timeout --kill-after=1s 5s getent ahosts "$host" >/dev/null 2>&1; then
            printf 'DNS resolved %s\n' "$host"
        else
            printf 'DNS lookup failed for %s (exit %d)\n' "$host" "$?"
        fi
    done <<< "$hosts"

    echo 'Refreshing APT indexes'
    run_apt -o APT::Update::Error-Mode=any update
    echo 'Installing CI packages'
    run_apt install --yes --no-install-recommends \
        build-essential cmake libopus-dev pkg-config postgresql-client
} 2>&1 | tee apt-install.log
