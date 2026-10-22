import { useRef } from "react";

/**
 * 导航期间的三道时序护栏。
 *
 * 它们此前是三个裸的 useRef 计数器，名字相近、语义完全不同，读代码时分不出哪个管哪件事。
 * 三件事分别是：
 *
 * 1. 启动恢复只能跑一次，而且用户先动过就不该再跑。本地服务要几秒才就绪，恢复要等它；
 *    这期间用户很可能已经自己点去别处了，恢复晚一步把页面抢回去是最恼人的一种 bug。
 * 2. 项目身份解析在途时用户可能已经离开这个项目。晚到的响应不能把他重新拽回去。
 * 3. 恢复分两部分：项目上下文和所在页面。用户在解析期间导航过的话，上下文照样恢复
 *    （它不改变用户看到的东西），但页面必须听用户的。
 *
 * 三者互相独立：exitProject 要作废在途解析，但用户点一下侧栏不该作废；
 * 而两者都该让启动恢复失效。
 */
export interface NavigationGuards {
  /** 用户显式导航到某一页。作废尚未执行的启动恢复，并记一次页面变更。 */
  userNavigated: () => void;
  /** 离开项目。作废在途的项目解析，同时作废尚未执行的启动恢复。 */
  projectExited: () => void;
  /** 开始一次项目身份解析，返回"这次解析结果是否仍然该被采纳"的判定。 */
  beginProjectResolve: () => () => boolean;
  /** 认领启动恢复。第一次返回 true，此后一律 false；用户已经导航过时也返回 false。 */
  claimRestore: () => boolean;
  /** 记下此刻的导航次数，返回"用户此后是否又导航过"的判定。 */
  watchUserNavigation: () => () => boolean;
}

export function useNavigationGuards(): NavigationGuards {
  const restoreClaimed = useRef(false);
  const resolveGeneration = useRef(0);
  const userNavigations = useRef(0);

  return useRef<NavigationGuards>({
    userNavigated: () => {
      restoreClaimed.current = true;
      userNavigations.current += 1;
    },
    projectExited: () => {
      resolveGeneration.current += 1;
      restoreClaimed.current = true;
    },
    beginProjectResolve: () => {
      const started = resolveGeneration.current;
      return () => started === resolveGeneration.current;
    },
    claimRestore: () => {
      if (restoreClaimed.current) return false;
      restoreClaimed.current = true;
      return true;
    },
    watchUserNavigation: () => {
      const started = userNavigations.current;
      return () => started !== userNavigations.current;
    },
  }).current;
}
