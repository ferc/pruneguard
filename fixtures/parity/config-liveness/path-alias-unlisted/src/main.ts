import { helper } from '@shared/utils';
import { Button } from '@Components/Button';
import { config } from '~/config';
import { store } from '$lib/store';
import _ from 'lodash';

export function app() {
  return helper() + Button + config + store + _.identity('ok');
}
