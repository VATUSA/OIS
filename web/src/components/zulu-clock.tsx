import {useEffect, useState} from "react";

function zulu(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}Z`;
}

/** Live current Zulu (UTC) time, `HH:MM:SSZ`, ticking each second. */
export function ZuluClock({className = ""}: {className?: string}) {
  const [t, setT] = useState(zulu);
  useEffect(() => {
    const id = setInterval(() => setT(zulu()), 1000);
    return () => clearInterval(id);
  }, []);
  return (
    <span
      className={`font-mono text-sm tabular-nums ${className}`}
      title="Current Zulu (UTC) time"
      aria-label="Current Zulu time"
    >
      {t}
    </span>
  );
}
