import { useI18n } from '../../i18n';
import { SettingsHelp } from '../SettingsHelp';

export function CoolingToggle({ disabledCooling, disabled, onChange }: {
  disabledCooling: boolean;
  disabled: boolean;
  onChange: (disabledCooling: boolean) => void;
}) {
  const { t } = useI18n();
  const label = t('config.network.enableCooling');
  return (
    <div className="config-network-field config-network-toggle">
      <div>
        <span className="config-field-label">
          <label htmlFor="config-enable-cooling">{label}</label>
          <SettingsHelp label={label}>{t('config.network.enableCoolingHint')}</SettingsHelp>
        </span>
      </div>
      <label className="switch-control" title={label}>
        <input id="config-enable-cooling" type="checkbox" role="switch" aria-label={label}
          checked={!disabledCooling} disabled={disabled}
          onChange={(event) => onChange(!event.currentTarget.checked)} />
        <span className="switch-track" />
      </label>
    </div>
  );
}
