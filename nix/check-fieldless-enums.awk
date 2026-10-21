function bare_type(type) {
  sub(/<.*/, "", type)
  sub(/^.*::/, "", type)
  return type
}

function record_trait(line, trait, marker, type) {
  marker = " core::" trait " for "
  if (index(line, marker) == 0) {
    return
  }
  type = line
  sub(/^.* for /, "", type)
  traits[bare_type(type), trait] = 1
}

{
  record_trait($0, "marker::Copy")
  record_trait($0, "cmp::Eq")
  record_trait($0, "hash::Hash")
  record_trait($0, "fmt::Debug")

  line = $0
  sub(/^#[^]]*] /, "", line)
  if (line ~ /^pub enum /) {
    type = line
    sub(/^pub enum /, "", type)
    sub(/<.*/, "", type)
    enum_count++
    enum_type[enum_count] = type
    enum_name[enum_count] = bare_type(type)
    fieldless[enum_count] = 1
    current = enum_count
    next
  }

  if (current && index(line, "pub " enum_type[current] "::") == 1) {
    variant = substr(line, length("pub " enum_type[current] "::") + 1)
    variant_count[current]++
    if (variant ~ /\(/ || variant ~ /::/) {
      fieldless[current] = 0
    }
    next
  }

  if (current && line ~ /^(pub |impl |#)/) {
    current = 0
  }
}

END {
  failed = 0
  for (i = 1; i <= enum_count; i++) {
    if (!fieldless[i] || !variant_count[i]) {
      continue
    }
    name = enum_name[i]
    missing = ""
    if (!traits[name, "marker::Copy"]) missing = missing " Copy"
    if (!traits[name, "cmp::Eq"]) missing = missing " Eq"
    if (!traits[name, "hash::Hash"]) missing = missing " Hash"
    if (!traits[name, "fmt::Debug"]) missing = missing " Debug"
    if (missing != "") {
      print enum_type[i] " is missing:" missing > "/dev/stderr"
      failed = 1
    }
  }
  exit failed
}
