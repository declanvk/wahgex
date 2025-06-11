build-web:
    ./scripts/build-web.sh

watch-web:
    watchexec --watch web just build-web

serve-web:
    python3 -m http.server --directory web/dist

clean-web:
    [ -d "web/dist" ] && rm -r "web/dist" || echo "web/dist does not exist"
