set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

root_dir := justfile_directory()
# Show available commands
default:
    @just --list

setup-qtbridge-rust:
    @just --justfile "{{justfile()}}" setup-target \
        "ext/qtbridge-rust" \
        "ext/qtbridge-rust" \
        "f7fe523d10357751bc90c334263188610c0382cf" \
        "https://github.com/qt/qtbridge-rust.git" \
        "" \
        ""


setup-target clone_dir target_dir repo_rev repo_url target_patch sparse_dirs:
    mkdir -p "{{root_dir}}/ext"
    git config --global core.autocrlf false
    if [ ! -d "{{root_dir}}/{{clone_dir}}/.git" ]; then \
      git clone --depth 1 --filter=blob:none --no-checkout "{{repo_url}}" "{{root_dir}}/{{clone_dir}}"; \
    else \
      echo "{{clone_dir}} already exists; skipping clone"; \
    fi
    cd "{{root_dir}}/{{clone_dir}}" && git fetch --depth 1 origin "{{repo_rev}}"
    if [ -n "{{sparse_dirs}}" ]; then \
      cd "{{root_dir}}/{{clone_dir}}" && git sparse-checkout init --cone; \
      cd "{{root_dir}}/{{clone_dir}}" && git sparse-checkout set {{sparse_dirs}}; \
    fi
    cd "{{root_dir}}/{{clone_dir}}" && git checkout --detach FETCH_HEAD
    test -d "{{root_dir}}/{{target_dir}}"
    if [ -n "{{target_patch}}" ]; then \
        cd "{{root_dir}}/{{target_dir}}" && git apply --check "{{root_dir}}/{{target_patch}}"; \
        cd "{{root_dir}}/{{target_dir}}" && git apply "{{root_dir}}/{{target_patch}}"; \
    fi

reset-target clone_dir target_dir repo_rev target_patch:
    test -d "{{root_dir}}/{{clone_dir}}/.git"
    cd "{{root_dir}}/{{clone_dir}}" && git fetch --depth 1 origin "{{repo_rev}}"
    cd "{{root_dir}}/{{clone_dir}}" && git reset --hard FETCH_HEAD
    cd "{{root_dir}}/{{clone_dir}}" && git clean -fd
    cd "{{root_dir}}/{{target_dir}}" && git apply --check "{{root_dir}}/{{target_patch}}"
    cd "{{root_dir}}/{{target_dir}}" && git apply "{{root_dir}}/{{target_patch}}"

setup-ext: setup-qtbridge-rust

# Build a self-contained macOS app (optionally specify a Rust target triple).
bundle-macos target="":
    bash "{{root_dir}}/scripts/package-macos.sh" "{{target}}"