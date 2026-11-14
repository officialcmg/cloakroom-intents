// /api/oneclick/v0/tokens: forwards the 1Click call of the same name (docs/ONECLICK_PROXY.md).
import { handleOneClick } from '../../../server/oneclickProxy.js';

export default {
  fetch(request: Request): Promise<Response> {
    return handleOneClick(request, 'tokens');
  },
};
