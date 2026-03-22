# ---------------------------------------------------------------------------
# nginx-logger demo
# ---------------------------------------------------------------------------

Output demo.gif

Set Theme "Dracula"
Set FontSize 14
Set Width 900
Set Height 550
Set Shell "bash"
Set Padding 20

# ---------------------------------------------------------------------------
# 1. Build the binary (quiet)
# ---------------------------------------------------------------------------
Type "cargo build -q 2>&1 && echo 'Build OK'"
Enter
Sleep 6s

# ---------------------------------------------------------------------------
# 2. Start the server in the background; capture its PID from the log line
# ---------------------------------------------------------------------------
Type "cargo run -q &"
Enter
Sleep 3s

Type "SERVER_PID=$!"
Enter
Sleep 500ms

# ---------------------------------------------------------------------------
# 3. Show server startup log (PID printed on startup)
# ---------------------------------------------------------------------------
Sleep 1s

# ---------------------------------------------------------------------------
# 4. GET / – fetch the index page (DEBUG level, so request is visible)
# ---------------------------------------------------------------------------
Type "curl -s -o /dev/null -w 'GET /  →  HTTP %{http_code}' http://localhost:3000/"
Enter
Sleep 1500ms

# ---------------------------------------------------------------------------
# 5. POST /add – store an entry
# ---------------------------------------------------------------------------
Type "curl -s -o /dev/null -w 'POST /add  →  HTTP %{http_code}' \\"
Enter
Type "  -d 'name=Alice&age=30' http://localhost:3000/add"
Enter
Sleep 1500ms

# ---------------------------------------------------------------------------
# 6. POST /add – second entry
# ---------------------------------------------------------------------------
Type "curl -s -o /dev/null -w 'POST /add  →  HTTP %{http_code}' \\"
Enter
Type "  -d 'name=Bob&age=25' http://localhost:3000/add"
Enter
Sleep 1500ms

# ---------------------------------------------------------------------------
# 7. Toggle log level DEBUG → INFO via SIGUSR1
# ---------------------------------------------------------------------------
Type "echo '--- toggling log level to INFO via SIGUSR1 ---'"
Enter
Sleep 500ms
Type "kill -USR1 $SERVER_PID"
Enter
Sleep 1500ms

# ---------------------------------------------------------------------------
# 8. GET / again – at INFO level the incoming request is no longer logged
# ---------------------------------------------------------------------------
Type "curl -s -o /dev/null -w 'GET /  →  HTTP %{http_code}' http://localhost:3000/"
Enter
Sleep 1500ms

# ---------------------------------------------------------------------------
# 9. Toggle back DEBUG → INFO → DEBUG
# ---------------------------------------------------------------------------
Type "echo '--- toggling back to DEBUG ---'"
Enter
Sleep 500ms
Type "kill -USR1 $SERVER_PID"
Enter
Sleep 1500ms

# ---------------------------------------------------------------------------
# 10. Run the test suite
# ---------------------------------------------------------------------------
Type "kill $SERVER_PID 2>/dev/null; cargo test -q 2>&1"
Enter
Sleep 8s

Sleep 2s

