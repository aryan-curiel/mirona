#!/bin/sh
# Builds the site (https://mirona.app, on Netlify) into target/site (NGR-178): the landing page at the root with the
# tokens, logo and screenshots it uses, and the docs site (scripts/build-docs.sh) under docs/.
# Without target/book the landing page still builds, for a quick look: open target/site/index.html.
set -eu
cd "$(dirname "$0")/.."
rm -rf target/site
mkdir -p target/site/assets
cp site/index.html target/site/
cp design-kit/tokens/tokens.css design-kit/brand/app-icon.svg design-kit/brand/mark-small.svg \
  docs/images/meeting.png docs/images/demo.gif target/site/assets/
if [ -d target/book ]; then cp -R target/book target/site/docs; fi
echo "site in target/site (open target/site/index.html)"
