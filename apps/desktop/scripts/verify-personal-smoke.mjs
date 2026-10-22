import { existsSync, readFileSync } from 'node:fs';
const file = process.argv[2];
if (!file) throw new Error('缺少个人启动检查日志');
const entries = readFileSync(file, 'utf8').split(/\r?\n/).filter(line => line.startsWith('[personal-check] '));
if (entries.length !== 1) throw new Error('个人启动检查未完成或结果不唯一');
const value = JSON.parse(entries[0].slice('[personal-check] '.length));
if (value.result !== 'passed' || value.platform !== process.platform || value.arch !== process.arch
  || value.memoryEnabled !== false || value.ui?.navigation?.join(',') !== '工作台,对话,项目,文件,自动化,技能,应用'
  || value.ui.login || value.ui.alerts?.length || !existsSync(value.screenshot)) throw new Error('个人启动检查结果不完整');
console.log('个人版无模型启动与七个主入口检查通过；完整首版能力另行验收');
