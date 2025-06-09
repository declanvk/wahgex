build-web:
    ./web/build.sh

watch-web:
    watchexec --watch ./web just build-web

serve-web:
    python3 -m http.server --directory ./web/dist
