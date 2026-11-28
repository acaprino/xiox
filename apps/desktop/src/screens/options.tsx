// The options of a section, as a form that saves itself: each control sends today's intent when it changes and draws what comes back
// (the page never keeps a value of its own but the text being typed in a number box). Which intent a control sends is `intents.ts`'s.
import { useState } from 'react';
import { Checkbox, Button, Column, Input, RadioGroup, RadioGroupItem, Text } from '@acaprino/sigrid/primitives';
import { FormField, ItemList, ItemRow, Note } from '@acaprino/sigrid/components';
import type { Controller } from '../controller.ts';
import { optionIntent } from '../intents.ts';
import type { OptionView, SectionId } from '../types.ts';
import { noteTone } from './notices.tsx';

type Of<K extends OptionView['kind']> = Extract<OptionView, { kind: K }>;
interface Props<K extends OptionView['kind']> { c: Controller; section: SectionId; option: Of<K> }

const send = (c: Controller, section: SectionId, option: OptionView, value: boolean | number | string | null): void => {
  void c.send(section, optionIntent(section, option, value));
};

function Toggle({ c, section, option }: Props<'toggle'>) {
  return (
    <Column gap="xs">
      <Checkbox label={option.label} checked={option.on} disabled={!option.enabled} onCheckedChange={() => send(c, section, option, !option.on)} />
      {option.hint ? <Text variant="muted">{option.hint}</Text> : null}
    </Column>
  );
}

/** A whole number in a range: sent when the box is left or Enter is pressed; a value outside the range says so and is put back. */
function Number_({ c, section, option }: Props<'number'>) {
  const [text, setText] = useState<string | null>(null);
  const shown = text ?? String(option.value);
  const value = Number(shown);
  const bad = shown.trim() === '' || !Number.isInteger(value) || value < option.min || value > option.max;
  const commit = (): void => {
    if (text === null) return;
    setText(null);
    if (!bad && value !== option.value) send(c, section, option, value);
  };
  return (
    <FormField label={option.label} error={text !== null && bad ? `Use a whole number from ${option.min} to ${option.max}.` : undefined}>
      <Input
        type="number" short="xs" min={option.min} max={option.max} step={1} value={shown} disabled={!option.enabled}
        onChange={(event) => setText(event.target.value)} onBlur={commit}
        onKeyDown={(event) => { if (event.key === 'Enter') commit(); }}
      />
    </FormField>
  );
}

function Choice({ c, section, option }: Props<'choice'>) {
  return (
    <FormField label={option.label}>
      <RadioGroup value={option.value} disabled={!option.enabled} onValueChange={(next) => send(c, section, option, next)}>
        {option.choices.map((choice) => <RadioGroupItem key={choice.id} value={choice.id}>{choice.label}</RadioGroupItem>)}
      </RadioGroup>
    </FormField>
  );
}

/** The extra folders: pick one, add one (the picker opens in Rust), remove the picked one. */
function Folders({ c, section, option }: Props<'list'>) {
  return (
    <Column gap="sm">
      <Text variant="strong">{option.label}</Text>
      {option.items.length === 0 ? <Text variant="muted">{option.empty}</Text> : (
        <ItemList aria-label={option.label}>
          {option.items.map((text, index) => (
            <ItemRow key={`${index}:${text}`} title={text} current={index === option.selected} disabled={!option.enabled} onClick={() => send(c, section, option, index)} />
          ))}
        </ItemList>
      )}
      <Button variant="outline" size="sm" disabled={!option.enabled} onClick={() => { void c.addRoot(); }}>{option.addLabel}</Button>
      <Button
        variant="outline" size="sm" disabled={!option.enabled || option.selected === null}
        title={option.enabled && option.selected === null ? 'Pick a folder in the list first.' : undefined}
        onClick={() => send(c, section, option, 'remove')}
      >{option.removeLabel}</Button>
    </Column>
  );
}

export function OptionControl({ c, section, option }: { c: Controller; section: SectionId; option: OptionView }) {
  switch (option.kind) {
    case 'toggle': return <Toggle c={c} section={section} option={option} />;
    case 'number': return <Number_ c={c} section={section} option={option} />;
    case 'choice': return <Choice c={c} section={section} option={option} />;
    case 'list': return <Folders c={c} section={section} option={option} />;
    case 'button': return <Button variant="outline" size="sm" disabled={!option.enabled} title={option.hint ?? undefined} onClick={() => send(c, section, option, null)}>{option.label}</Button>;
    case 'text': return <Note tone={noteTone(option.tone)} title={option.text} />;
  }
}
