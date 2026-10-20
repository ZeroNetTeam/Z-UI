import { useEffect } from "react";
import { useSearchParams } from "react-router";

/**
 * Open a page's create dialog when the page is reached with `?new=<key>`
 * (the dashboard's Quick actions do), and take the mark off the address so a
 * reload does not open it again.
 */
export function useOpenFromLink(key: string, open: () => void) {
  const [params, setParams] = useSearchParams();
  useEffect(() => {
    if (params.get("new") !== key) return;
    open();
    setParams({}, { replace: true });
    // Once, on arrival.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
