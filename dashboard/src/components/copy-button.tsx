import { CheckIcon, CopyIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { copyAndTell } from "@/lib/clipboard";

/** Put `text` on the clipboard and show that it got there. */
export function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      onClick={() => {
        void copyAndTell(text).then((done) => done && setCopied(true));
      }}
    >
      {copied ? <CheckIcon /> : <CopyIcon />} {label}
    </Button>
  );
}
