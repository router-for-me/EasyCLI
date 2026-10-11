import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { SelectMenu } from '../../../src/components/SelectMenu';
import '../../../src/styles/index.css';
import './style.css';
function Fixture() {
  const [value, setValue] = useState('0');
  return <><div className="fixture-scroll"><div className="fixture-container">
    {Array.from({ length: 5 }, (_, index) => <SelectMenu key={index} value={value} onChange={setValue}
      ariaLabel={`Filter ${index}`} options={Array.from({ length: 80 }, (_, i) => ({ value: String(i), label: `Option ${i}`, group: `Group ${Math.floor(i / 20)}` }))} />)}
    <div className="fixture-spacer" />
  </div></div><button id="outside">Outside</button><output>{value}</output></>;
}
createRoot(document.getElementById('root')!).render(<Fixture />);
