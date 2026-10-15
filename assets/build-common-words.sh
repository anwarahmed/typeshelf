#!/bin/sh
# Regenerates assets/common-words.txt: the words the glossary treats as common and leaves
# out. They are SCOWL's two most common size levels (10 and 20) in every spelling
# variant, which include inflected forms ("horses", "wandered").
#
#   curl -LO https://downloads.sourceforge.net/wordlist/scowl-2020.12.07.tar.gz
#   tar xzf scowl-2020.12.07.tar.gz
#   assets/build-common-words.sh scowl-2020.12.07 > assets/common-words.txt
set -eu
scowl=${1:?usage: build-common-words.sh <unpacked scowl directory>}
cat <<'NOTICE'
# Common English words, from SCOWL (Spell Checker Oriented Word Lists) 2020.12.07,
# size levels 10 and 20: http://wordlist.aspell.net/ - built by assets/build-common-words.sh.
#
# Copyright 2000-2018 by Kevin Atkinson
#
# Permission to use, copy, modify, distribute and sell these word
# lists, the associated scripts, the output created from the scripts,
# and its documentation for any purpose is hereby granted without fee,
# provided that the above copyright notice appears in all copies and
# that both that copyright notice and this permission notice appear in
# supporting documentation. Kevin Atkinson makes no representations
# about the suitability of this array for any purpose. It is provided
# "as is" without express or implied warranty.
#
# SCOWL's other sources are credited in its Copyright file.
NOTICE
for v in english american british british_z canadian australian; do
  for size in 10 20; do
    f="$scowl/final/$v-words.$size"
    [ -f "$f" ] && cat "$f"
  done
done | iconv -f ISO-8859-1 -t UTF-8 | tr 'A-Z' 'a-z' | LC_ALL=C sort -u
