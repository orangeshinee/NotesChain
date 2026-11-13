#!/usr/bin/env bash
# 发布闸门（Linux / macOS）：一条命令跑完 CI 的 Linux 契约 + 服务只读检查。
# 用法：bash tools/release-gate.sh [-p 端口]
#
# 与 Windows 版 tools/release-gate.ps1 的关系：
#   - 本脚本覆盖 .github/workflows/ci.yml 的 check job（Linux 侧）全部四项，
#     外加一次真实服务启动 + 只读接口检查。
#   - ps1 的边界场景、平替预演、浏览器探针依赖 pwsh 与 Windows 路径，
#     Linux 版不跑；前端交互验证需另用浏览器脚本或手工验证。
#   - 前端资源是编译期内嵌的，所以 build 前额外校验 JS 语法与 CSS 括号平衡，
#     避免语法错误被内嵌进二进制却只在运行时暴露。
#   - 服务检查用隔离临时目录 + 唯一哨兵文件，断言响应确实来自本次启动的二进制，
#     否则端口被占用时会把别的实例当成被测对象（假绿）。

set -uo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo" || exit 1

PORT=18099
while [ $# -gt 0 ]; do
  case "$1" in
    -p|--port)
      if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
        echo "缺少端口值：-p <端口>" >&2
        exit 2
      fi
      PORT="$2"; shift 2 ;;
    -h|--help) echo "用法: bash tools/release-gate.sh [-p 端口]"; exit 0 ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

pass=0
fail=0
record() {
  local name="$1" ok="$2" detail="${3:-}"
  if [ "$ok" -eq 0 ]; then
    printf '  [PASS] %-28s %s\n' "$name" "$detail"
    pass=$((pass + 1))
  else
    printf '  [FAIL] %-28s %s\n' "$name" "$detail"
    fail=$((fail + 1))
  fi
}

# 输出重定向到临时日志再判退出码，避免管道截断造成 SIGPIPE 假失败
TMP="$(mktemp -d)"
if [ -z "$TMP" ] || [ ! -d "$TMP" ]; then
  echo "无法创建临时目录（TMPDIR 不可写？）" >&2
  exit 1
fi
srv_pid=""
cleanup_done=0
cleanup() {
  [ "$cleanup_done" -eq 1 ] && return
  cleanup_done=1
  if [ -n "$srv_pid" ] && kill -0 "$srv_pid" 2>/dev/null; then
    kill "$srv_pid" 2>/dev/null
    wait "$srv_pid" 2>/dev/null
  fi
  [ -n "$TMP" ] && [ -d "$TMP" ] && rm -rf "$TMP"
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT TERM

echo "=== 0. 前端静态检查 ==="
node --check web/app.js > "$TMP/js.log" 2>&1
record "node --check web/app.js" $? "$(tail -1 "$TMP/js.log")"
python3 - "$TMP" <<'PY' > "$TMP/css.log" 2>&1
import sys
s = open('web/style.css', encoding='utf-8').read()
a, b = s.count('{'), s.count('}')
print(f'braces {a}/{b}')
sys.exit(0 if a == b else 1)
PY
record "CSS 括号平衡" $? "$(cat "$TMP/css.log")"

echo "=== 1. 构建（CI 契约，--locked） ==="
cargo build --release --locked > "$TMP/build.log" 2>&1
code=$?
exe=target/release/noteschain
if [ "$code" -eq 0 ] && [ ! -x "$exe" ]; then code=1; fi
record "cargo build --release" "$code" "exit=$code $(tail -1 "$TMP/build.log")"

echo "=== 2. 单元测试（CI 契约） ==="
cargo test --locked > "$TMP/test.log" 2>&1
code=$?
line="$(grep -E 'test result: ok\.' "$TMP/test.log" | tail -1)"
if [ "$code" -eq 0 ] && [ -z "$line" ]; then code=1; fi
record "cargo test" "$code" "exit=$code ${line:-（未找到 test result 行）}"

echo "=== 3. clippy 零警告（CI 契约，-D warnings） ==="
cargo clippy --all-targets --locked -- -D warnings > "$TMP/clippy.log" 2>&1
code=$?
record "cargo clippy" "$code" "exit=$code $(grep -cE '^(warning|error)' "$TMP/clippy.log") warnings/errors"

echo "=== 4. 二进制可用性（CI 契约） ==="
"$exe" --version > "$TMP/ver.log" 2>&1
code=$?
record "noteschain --version" "$code" "$(cat "$TMP/ver.log")"
"$exe" --help > /dev/null 2>&1
code=$?
record "noteschain --help" "$code" "exit=$code"

echo "=== 5. 服务只读检查（隔离临时目录 + 哨兵） ==="
notes="$TMP/notes"
sentinel="gate-sentinel-$$.md"
mkdir -p "$notes/raw" || exit 1
printf '# gate sentinel\n' > "$notes/raw/$sentinel"
"$exe" --dir "$notes" --port "$PORT" --no-token --no-git > "$TMP/server.log" 2>&1 &
srv_pid=$!
ready=0
for _ in $(seq 1 40); do
  # 本进程若已退出（例如端口被占用导致 bind 失败），立刻停止等待
  if ! kill -0 "$srv_pid" 2>/dev/null; then break; fi
  if curl -fsS "http://127.0.0.1:$PORT/api/files" > "$TMP/files.json" 2>/dev/null; then ready=1; break; fi
  sleep 0.25
done

# curl 200 只说明「有人答话」，不代表是本次构建的二进制。
# 三重确认：进程存活、无 bind 错误、响应含本次哨兵。
alive=0
kill -0 "$srv_pid" 2>/dev/null && alive=1
bind_err=0
grep -qiE 'address already in use|os error 98' "$TMP/server.log" && bind_err=1
if [ "$ready" -eq 1 ] && [ "$alive" -eq 1 ] && [ "$bind_err" -eq 0 ]; then
  record "被测服务进程存活" 0 "pid=$srv_pid port=$PORT"
else
  record "被测服务进程存活" 1 "ready=$ready alive=$alive bind_err=$bind_err $(tail -2 "$TMP/server.log" | tr '\n' ' ')"
fi

# 决定性断言：哨兵只存在于本次临时目录，别的实例不可能回显它
python3 - "$TMP/files.json" "$sentinel" <<'PY' > "$TMP/sentinel.log" 2>&1
import json, sys
with open(sys.argv[1], encoding='utf-8') as f:
    raw = f.read()
d = json.loads(raw)
assert 'groups' in d, 'missing groups'
assert sys.argv[2] in raw, 'sentinel missing: response did not come from this instance'
print('groups =', [g.get('kind') for g in d['groups']], '| sentinel =', sys.argv[2])
PY
record "sentinel 响应来自本实例" $? "$(cat "$TMP/sentinel.log")"

curl -fsS "http://127.0.0.1:$PORT/api/config" > "$TMP/config.json" 2>/dev/null
code=$?
record "/api/config" "$code" "exit=$code"
curl -fsS "http://127.0.0.1:$PORT/" > "$TMP/index.html" 2>/dev/null
code=$?
has_cal=0
[ "$code" -eq 0 ] && grep -q 'calendar-mount' "$TMP/index.html" && has_cal=1
record "首页内嵌 calendar-mount" $((1 - has_cal)) "found=$has_cal"

echo
echo "===== 汇总 ====="
echo "通过 $pass 项，失败 $fail 项"
if [ "$fail" -gt 0 ]; then
  echo "未通过，详见上方 [FAIL] 项"
  exit 1
fi
echo "全部通过"
exit 0
