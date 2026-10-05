#!/usr/bin/env bash
set -euo pipefail
mode=${NOCTERM_PERF_MODE:-$(cat /tmp/perf-mode 2>/dev/null || echo idle)}
fill() {
    for ((i=1; i<=80; i++)); do
        printf '\e[3%dm%-8s\e[0m \e[1mdrwxr-xr-x\e[0m  2 user user 4096 Oct  5 20:30 \e[34m/usr/share/long/path/component-%04d\e[0m trailing words\r\n' "$((i%7+1))" "f$i" "$i"
    done
}
sleep 2
fill
if [[ -n ${NOCTERM_PERF_RUN:-} ]]; then
    touch "$NOCTERM_PERF_RUN/shell-ready"
fi
case $mode in
    type)
        while :; do
            printf '\e[32m➜\e[0m \e[36m~/dev/pet\e[0m '
            text='cargo build --workspace --all-targets and more'
            for ((i=0; i<${#text}; i++)); do
                printf '%s' "${text:i:1}"
                sleep 0.066
            done
            printf '\r\n'
        done ;;
    flood)
        : "${NOCTERM_PERF_RUN:?flood needs a run directory}"
        sleep 5
        touch "$NOCTERM_PERF_RUN/flood-start"
        python3 - <<'PY'
import sys
for i in range(400000):
    sys.stdout.write(f'{i} some text in a line of output \x1b[31mred\x1b[0m\r\n')
sys.stdout.write('\x1b]2;NOCTERM-PERF-DRAINED\x07')
sys.stdout.flush()
PY
        touch "$NOCTERM_PERF_RUN/flood-produced"
        sleep 100000 ;;
    idle) sleep 100000 ;;
    shell) PS1='\[\e[32m\]➜ \[\e[36m\]\w\[\e[0m\] ' exec bash --norc -i ;;
esac
