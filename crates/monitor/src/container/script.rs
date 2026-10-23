//! Linux cgroup accounting only. CPU is explicitly measured against one CPU.
use crate::{MetricSet, MonitorMetric};
use nocterm_session::ExecRequest;
use std::time::Duration;

const RESOLVE: &str = r#"resolve() {
awk -v controller="$1" '
function has(list,wanted){return index("," list ",","," wanted ",")!=0}
function path(value){gsub(/\\040/," ",value); return value}
FILENAME=="/proc/self/cgroup" {
 if(length($0)>8192 || ++members>256) exit 1
 first=index($0,":"); rest=substr($0,first+1); second=index(rest,":")
 controllers=substr(rest,1,second-1); name=substr(rest,second+1)
 if((controller=="v2" && substr($0,1,first-1)=="0" && controllers=="") || has(controllers,controller)) {
  member=name; memberships++
 }
 next
}
{
 if(length($0)>16384 || ++mounts>4096) exit 1
 parts=split($0,halves," - "); if(parts!=2) next
 split(halves[2],post," "); split(halves[1],pre," ")
 if((controller=="v2" && post[1]=="cgroup2") || (post[1]=="cgroup" && has(post[3],controller))) {
  root=path(pre[4]); mount=path(pre[5]); matches++
 }
}
END {
 if(memberships!=1 || matches!=1 || member!~/^\// || root!~/^\// || mount!~/^\//) exit 1
 if(member~/\\/ || root~/\\/ || mount~/\\/ || member~/(^|\/)\.\.?($|\/)/) exit 1
 if(root=="/") relative=member
 else if(member==root) relative="/"
 else if(index(member,root "/")==1) relative=substr(member,length(root)+1)
 else relative=member
 if(relative=="/") relative=""
 printf "%s%s\n",mount,relative
}' /proc/self/cgroup /proc/self/mountinfo
}
member() {
 [ -r "$1/cgroup.procs" ] || return 1
 n=0
 while read -r pid; do
  [ "$pid" = "$$" ] && return 0
  n=$((n+1)); [ "$n" -le 4096 ] || return 1
 done <"$1/cgroup.procs"
 return 1
}
V2=$(resolve v2) || V2=
CPU=; MEM=; CV=1; MV=1
if [ -n "$V2" ] && member "$V2"; then
 [ ! -r "$V2/cpu.stat" ] || { CPU=$V2; CV=2; }
 [ ! -r "$V2/memory.current" ] || { MEM=$V2; MV=2; }
fi
if [ -z "$CPU" ]; then CPU=$(resolve cpuacct) || CPU=; [ -z "$CPU" ] || member "$CPU" || CPU=; fi
if [ -z "$MEM" ]; then MEM=$(resolve memory) || MEM=; [ -z "$MEM" ] || member "$MEM" || MEM=; fi
"#;
const CPU: &str = r#"if [ -n "$CPU" ] && member "$CPU"; then
 if [ "$CV" = 2 ]; then
  n=0; while read -r k v; do n=$((n+1)); [ "$n" -le 64 ] || break; [ "$k" != usage_usec ] || printf 'cpu_us %s\n' "$v"; done <"$CPU/cpu.stat"
 else read -r v <"$CPU/cpuacct.usage" && printf 'cpu_ns %s\n' "$v"
 fi
fi
"#;
const MEMORY: &str = r#"if [ -n "$MEM" ] && member "$MEM"; then
 if [ "$MV" = 2 ]; then
  read -r v <"$MEM/memory.current" && printf 'memory %s\n' "$v"
  read -r v <"$MEM/memory.max" && printf 'limit %s\n' "$v"
 else
  read -r v <"$MEM/memory.usage_in_bytes" && printf 'memory %s\n' "$v"
  read -r v <"$MEM/memory.limit_in_bytes" && printf 'limit_v1 %s\n' "$v"
 fi
fi
"#;
const PROOF: &str = r#"proof() {
printf '@@cgroup\n'
awk 'NR>256 || length($0)>8192 {print "invalid";exit} {print}' /proc/self/cgroup
printf '@@mountinfo\n'
awk 'NR>4096 || length($0)>16384 {print "invalid";exit} {print}' /proc/self/mountinfo
printf '@@selected\ncpu_path %s\nmemory_path %s\ncpu_version %s\nmemory_version %s\n' "$CPU" "$MEM" "$CV" "$MV"
}
"#;
pub(super) fn request(metrics: MetricSet, interval: Duration) -> ExecRequest {
    let mut source = String::from("{\nexec </dev/null 2>/dev/null\nexport LC_ALL=C\n");
    source.push_str(RESOLVE);
    source.push_str(PROOF);
    source.push_str(
        "f() {\nproof
printf '@@container\\n'\nread -r clock _ </proc/uptime && printf 'clock %s\\n' \"$clock\"\n",
    );
    if metrics.any(&[MonitorMetric::Cpu]) {
        source.push_str(CPU);
    }
    if metrics.any(&[MonitorMetric::Memory]) {
        source.push_str(MEMORY);
    }
    source.push_str("printf '@@end\\n' || exit\n}\nf; sleep 1\nwhile :; do f; sleep ");
    source.push_str(&interval.as_secs().max(1).to_string());
    source.push_str("; done\n}\n");
    ExecRequest::new("sh").arg("-s").stdin(source)
}
