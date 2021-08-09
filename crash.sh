set -euxo pipefail

git fetch --unshallow

git checkout bb19d024ea1e5e0c9a3d75a9ee58ff03c70c7e5d
cargo build --verbose --target aarch64-apple-darwin

git checkout 11e0cf842628685036265a6164295dd18d543978
cargo build --verbose --target aarch64-apple-darwin
