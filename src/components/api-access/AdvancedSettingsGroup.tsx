import { useId, type ReactNode } from 'react';
import './advanced-settings.css';

export function AdvancedSettingsGroup({ title, children }: { title: string; children: ReactNode }) {
  const titleId = useId();
  return (
    <section className="provider-settings-group" aria-labelledby={titleId}>
      <h3 id={titleId}>{title}</h3>
      <div className="provider-settings-group-content">{children}</div>
    </section>
  );
}
