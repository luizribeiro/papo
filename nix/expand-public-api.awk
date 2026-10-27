function declaration_path(line, value, fields) {
  value = line
  sub(/^#\[non_exhaustive\] /, "", value)
  if (value !~ /^pub (struct|enum|union|trait|type) /) {
    return ""
  }
  if (value ~ /^pub type .*>::/) {
    return ""
  }
  sub(/^pub (struct|enum|union|trait|type) /, "", value)
  split(value, fields, /[^[:alnum:]_:]/)
  sub(/:$/, "", fields[1])
  return fields[1]
}

function terminal_name(path, segments, count) {
  count = split(path, segments, /::/)
  return segments[count]
}

function belongs_to_path(line, path, value) {
  value = line
  sub(/^#\[non_exhaustive\] /, "", value)
  if (declaration_path(line) == path || index(value, "pub " path "::") == 1 ||
      index(value, "pub fn " path "::") == 1 || index(value, "pub const fn " path "::") == 1 ||
      index(value, "pub type " path "::") == 1) {
    return 1
  }
  if (line !~ /^impl/) {
    return 0
  }
  if (index(line, path " for ") != 0 || index(line, "for " path) != 0 ||
      index(line, "for &" path) != 0 || index(line, "for &mut " path) != 0) {
    return 1
  }
  if (length(line) >= length(path) && substr(line, length(line) - length(path) + 1) == path) {
    return 1
  }
  return 0
}

function replace_path(line, source, target) {
  gsub(source "::", target "::", line)
  gsub(source "<", target "<", line)
  gsub(source "\\(", target "(", line)
  gsub(source "\\)", target ")", line)
  gsub(source ",", target ",", line)
  gsub(source ">", target ">", line)
  gsub(source "\\]", target "]", line)
  gsub(source " ", target " ", line)
  gsub(source "&", target "&", line)
  gsub(source ":", target ":", line)
  if (length(line) >= length(source) && substr(line, length(line) - length(source) + 1) == source) {
    line = substr(line, 1, length(line) - length(source)) target
  }
  return line
}

function map_line(line, source) {
  for (source in selected_target) {
    line = replace_path(line, source, selected_target[source])
  }
  return line
}

FILENAME != "-" {
  path = declaration_path($0)
  if (path != "") {
    current = path
    name = terminal_name(path)
    if (!candidate_seen[name, path]) {
      candidate_seen[name, path] = 1
      candidate_count[name]++
      candidate[name, candidate_count[name]] = path
    }
    block[path] = $0
    next
  }
  if (current != "" && belongs_to_path($0, current)) {
    block[current] = block[current] ORS $0
  }
  next
}

{
  facade_count++
  facade[facade_count] = $0
  if ($0 ~ /^pub use papo::/) {
    target = substr($0, length("pub use ") + 1)
    target_for[terminal_name(target)] = target
    reexport_count++
    reexport[reexport_count] = target
  }
}

END {
  preferred["papo::Account"] = "papo_core::Account"
  preferred["papo::Away"] = "papo_core::Away"
  preferred["papo::Cap"] = "papo_core::Cap"
  preferred["papo::InvalidUrl"] = "papo_core::InvalidUrl"
  preferred["papo::ModeChange"] = "papo_core::ModeChange"

  for (name in target_for) {
    target = target_for[name]
    chosen = ""
    for (candidate_index = 1; candidate_index <= candidate_count[name]; candidate_index++) {
      source = candidate[name, candidate_index]
      if (source == target || source == preferred[target]) {
        chosen = source
      }
    }
    if (chosen == "" && candidate_count[name] == 1) {
      chosen = candidate[name, 1]
    }
    if (chosen == "" && candidate_count[name] > 1) {
      print "ambiguous public API source for " target > "/dev/stderr"
      failed = 1
    } else if (chosen != "") {
      selected_source[target] = chosen
      selected_target[chosen] = target
    }
  }
  if (failed) {
    exit 1
  }

  selected_target["papo_core::cap::Cap"] = "papo::Cap"
  selected_target["papo_proto::modes::ModeAction"] = "papo::ModeAction"

  for (line_number = 1; line_number <= facade_count; line_number++) {
    print facade[line_number]
  }
  print "# re-exported type surfaces"
  for (reexport_index = 1; reexport_index <= reexport_count; reexport_index++) {
    target = reexport[reexport_index]
    source = selected_source[target]
    if (source == "" || emitted[source]) {
      continue
    }
    emitted[source] = 1
    count = split(block[source], lines, ORS)
    for (block_index = 1; block_index <= count; block_index++) {
      print map_line(lines[block_index])
    }
  }
}
