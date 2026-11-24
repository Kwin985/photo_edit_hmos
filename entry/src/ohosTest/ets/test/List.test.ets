import abilityTest from './Ability.test';
import engineTest from './Engine.test';
import documentTest from './Document.test';
import advancedTest from './Advanced.test';
import perfTest from './Perf.test';

export default function testsuite() {
  abilityTest();
  engineTest();
  documentTest();
  advancedTest();
  perfTest();
}