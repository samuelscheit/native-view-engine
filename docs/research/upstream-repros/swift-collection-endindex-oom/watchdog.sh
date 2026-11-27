#!/usr/bin/env bash
# usage: watchdog.sh <rss_cap_kb> <timeout_s> -- cmd...
# Runs cmd; kills its whole process group once any descendant RSS exceeds cap or time runs out.
cap=$1; tmo=$2; shift 3
"$@" & pid=$!
start=$SECONDS; peak=0; reason=ok
while kill -0 $pid 2>/dev/null; do
  kids=$(pgrep -P $pid | tr '\n' ' ')
  for k in $kids; do kids="$kids $(pgrep -P $k | tr '\n' ' ')"; done
  rss=0; for p in $pid $kids; do r=$(ps -o rss= -p $p 2>/dev/null); [ -n "$r" ] && [ "$r" -gt "$rss" ] && rss=$r; done
  [ "$rss" -gt "$peak" ] && peak=$rss
  if [ "$rss" -gt "$cap" ]; then reason="KILLED rss>${cap}KB"; fi
  if [ $((SECONDS-start)) -ge "$tmo" ]; then reason="KILLED timeout ${tmo}s"; fi
  if [ "$reason" != ok ]; then
    [ -n "$WATCHDOG_SAMPLE" ] && for p in $kids; do ps -o comm= -p $p | grep -q swift-frontend && sample $p 3 -file "$WATCHDOG_SAMPLE" >/dev/null 2>&1; done
    kill -9 $kids $pid 2>/dev/null; break
  fi
  sleep 0.5
done
wait $pid 2>/dev/null; rc=$?
echo "result=$reason exit=$rc elapsed=$((SECONDS-start))s peak_rss=$((peak/1024))MB"
