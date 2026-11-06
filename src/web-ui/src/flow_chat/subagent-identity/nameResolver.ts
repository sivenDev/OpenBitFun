import { hashSubagentIdentity } from './identityHash';

// Do not reorder or grow this versioned pool: restored sessions must retain
// their identities in every locale. Core resolves the same names for the model
// (`agentic::subagent_names`), so both read them from shared terms.
export const SUBAGENT_NAME_KEYS = [
  'shared:subagentNames.bouncyBean',
  'shared:subagentNames.cheekyBao',
  'shared:subagentNames.sneakyMochi',
  'shared:subagentNames.zoomingPotato',
  'shared:subagentNames.curiousOtter',
  'shared:subagentNames.discoPenguin',
  'shared:subagentNames.noodleNinja',
  'shared:subagentNames.bubbleBandit',
  'shared:subagentNames.sleepyTaco',
  'shared:subagentNames.pixelRascal',
  'shared:subagentNames.jellyRocket',
  'shared:subagentNames.wobblyPudding',
  'shared:subagentNames.waffleWizard',
  'shared:subagentNames.sneezingDragon',
  'shared:subagentNames.giggleFox',
  'shared:subagentNames.turboSnail',
  'shared:subagentNames.marshmallowPirate',
  'shared:subagentNames.pogoPanda',
  'shared:subagentNames.toastDetective',
  'shared:subagentNames.zigzagDuck',
  'shared:subagentNames.mangoGoblin',
  'shared:subagentNames.biscuitBandit',
  'shared:subagentNames.cometHamster',
  'shared:subagentNames.captainDucky',
  'shared:subagentNames.hiccupSeal',
  'shared:subagentNames.popcornImp',
  'shared:subagentNames.wobbleCorgi',
  'shared:subagentNames.ninjaPeach',
  'shared:subagentNames.cloudHopper',
  'shared:subagentNames.moonBunny',
] as const;

export function resolveSubagentNameKey(identity: string): typeof SUBAGENT_NAME_KEYS[number] {
  const seed = identity.trim();
  if (!seed) return SUBAGENT_NAME_KEYS[0];
  return SUBAGENT_NAME_KEYS[hashSubagentIdentity(`subagent-names-v1:${seed}`) % SUBAGENT_NAME_KEYS.length];
}
