import { useState } from 'react';
import { Puzzle } from 'lucide-react';
import { safePluginWebURL } from '../services/pluginResources';

/** Registry/metadata artwork is optional. Invalid or unavailable images keep the fallback. */
export function PluginCardIcon({ logo }: { logo: string }) {
  const url = safePluginWebURL(logo);
  const [failedURL, setFailedURL] = useState('');
  return <span className="plugin-icon" aria-hidden="true">
    {url && failedURL !== url
      ? <img src={url} alt="" referrerPolicy="no-referrer" onError={() => setFailedURL(url)} />
      : <Puzzle size={23} />}
  </span>;
}
