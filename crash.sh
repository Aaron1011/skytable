set -euxo pipefail

git checkout v0.7.0-alpha.2
make test

git checkout 11e0cf842628685036265a6164295dd18d543978
make test
