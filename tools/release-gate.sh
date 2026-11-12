#!/usr/bin/env bash
# 发布闸门（Linux / macOS）：一条命令跑完 CI 的 Linux 契约 + 服务接口冒烟。
# 用法：bash tools/release-gate.sh [-p 端口]
#
# 与 Windows 版 tools/release-gate.ps1 的关系：
#   - 本脚本覆盖 .github/workflows/ci.yml 的 check job（Linux 侧）全部四项，
#     外加一次真实服务启动 + 接口冒烟。
#   - ps1 里的「边界场景 18 项 / 平替预演 20 项 / 浏览器交互」依赖 pwsh 与
#     Windows 路径，Linux 上不跑；浏览器验证用 playwright 单独做。
#   - 前端资源是编译期内嵌的，所以 build 前额外校验 JS 语法与 CSS 括号平衡，
#     避免语法错误被内嵌进二进制却只在运行时暴露。

set -uo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo" || exit 1

PORT=18099
while [ $# -gt 0 ]; do
  case "$1" in
    -p|--port) PORT="${2:-18099}"; shift 2 ;;
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
srv_pid=""
cleanup() {
  if [ -n "$srv_pid" ] && kill -0 "$srv_pid" 2>/dev/null; then
    kill "$srv_pid" 2>/dev/null
    wait "$srv_pid" 2>/dev/null
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

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
record "noteschain --help" $? "exit=$?"

echo "=== 5. 服务接口冒烟（临时目录，不碰真实笔记库） ==="
notes="$TMP/notes"
mkdir -p "$notes/raw" || exit 1
"$exe" --dir "$notes" --port "$PORT" --no-token --no-git > "$TMP/server.log" 2>&1 &
srv_pid=$!
ready=0
for _ in $(seq 1 40); do
  if curl -fsS "http://127.0.0.1:$PORT/api/files" > "$TMP/files.json" 2>/dev/null; then ready=1; break; fi
  sleep 0.25
done
if [ "$ready" -eq 1 ]; then
  record "服务启动 + /api/files" 0 "port=$PORT"
  python3 - "$TMP/files.json" <<'PY' > "$TMP/json.log" 2>&1
import json, sys
with open(sys.argv[1], encoding='utf-8') as f:
    d = json.load(f)
assert 'groups' in d, 'missing groups'
print('groups =', [g.get('kind') for g in d['groups']])
PY
  record "/api/files JSON 结构" $? "$(cat "$TMP/json.log")"
  curl -fsS "http://127.0.0.1:$PORT/api/config" > "$TMP/config.json" 2>/dev/null
  record "/api/config" $? "exit=$?"
  curl -fsS "http://127.0.0.1:$PORT/" > "$TMP/index.html" 2>/dev/null
  code=$?
  has_cal=0
  [ "$code" -eq 0 ] && grep -q 'calendar-mount' "$TMP/index.html" && has_cal=1
  record "首页内嵌 calendar-mount" $((1 - has_cal)) "found=$has_cal"
else
  record "服务启动 + /api/files" 1 "$(tail -3 "$TMP/server.log" | tr '\n' ' ')"
fi

echo
echo "===== 汇总 ====="
echo "通过 $pass 项，失败 $fail 项"
if [ "$fail" -gt 0 ]; then
  echo "未通过，详见上方 [FAIL] 项"
  exit 1
fi
echo "全部通过"
exit 0
