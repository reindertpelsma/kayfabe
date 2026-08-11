#!/usr/bin/env bash
# the w258 gate's OWN enumerator, lifted verbatim
cd "$1" || exit 2
find crates -name '*.rs' -not -path '*/tests/*' -not -path '*/benches/*' 2>/dev/null | sort -u | while read -r f; do
  awk -v F="$f" '
    /^[[:space:]]*(pub )?trait [A-Za-z]/ { intrait=1; depth=0 }
    intrait { depth += gsub(/{/,"{"); depth -= gsub(/}/,"}"); if (depth<=0 && NR>1) intrait=0; next }
    /^[[:space:]]*#\[/ { if ($0 ~ /no_mangle/ || $0 ~ /export_name/) ffi=1; next }
    /^[[:space:]]*pub[[:space:]]/ {
      head=$0; sub(/[(<].*$/,"",head)
      if (head ~ /[[:space:]]fn[[:space:]]+[a-z_][A-Za-z0-9_]*[[:space:]]*$/) {
        name=head; sub(/^.*[[:space:]]fn[[:space:]]+/,"",name); gsub(/[[:space:]]/,"",name)
        print F "\t" NR "\t" name "\t" (ffi ? "FFI" : "RS")
      }
      ffi=0; next
    }
    /^[[:space:]]*$/ { ffi=0 }
  ' "$f"
done
