<script setup lang="ts">
import { shallowRef } from 'vue'
import { version } from '../../package.json'
import appIcon from '../assets/app-icon.png'
import AppIcon from './AppIcon.vue'
import UpdatePanel from './UpdatePanel.vue'
import { followProjectLink, projectLinks, type ProjectLink } from '../transport/project-links'

const error = shallowRef('')
const failedLink = shallowRef<ProjectLink>('repository')
const links = [
  { id: 'repository', icon: 'github', title: '开源仓库', detail: '查看源代码，参与项目建设' },
  { id: 'guide', icon: 'book', title: '使用指南', detail: '从首次配对到显示器配置' },
  { id: 'releases', icon: 'download', title: '版本更新', detail: '查看更新内容与下载安装包' },
  { id: 'feedback', icon: 'message', title: '反馈与建议', detail: '报告问题，分享你的使用体验' },
] as const

async function open(event: MouseEvent, link: ProjectLink) {
  error.value = ''
  try {
    await followProjectLink(event, link)
  } catch {
    failedLink.value = link
    error.value = '暂时无法打开浏览器，请复制下方链接后访问。'
  }
}
</script>

<template>
  <section class="about-page" aria-labelledby="about-title">
    <div class="page-heading">
      <div>
        <h1 id="about-title">关于我们</h1>
        <p>让两台电脑，共享一个顺手的工作空间。</p>
      </div>
    </div>

    <div class="panel about-intro">
      <div class="about-identity">
        <img class="about-logo" :src="appIcon" alt="" width="76" height="76" />
        <div>
          <h2 class="about-name">Deskflow DDC</h2>
          <p class="about-tagline">一屏双机 · 自由切换</p>
          <div class="about-badges">
            <span class="about-badge copyable-text">v{{ version }}</span>
            <span class="about-badge">MIT 开源</span>
          </div>
        </div>
      </div>
      <div class="about-story">
        <p>
          少一次拔插，少一次伸手切换。Deskflow DDC
          将显示器输入源切换与键鼠共享放在一起，让你在两台电脑之间更从容地工作。
        </p>
        <p>
          你可以在局域网内共享键鼠、剪贴板与文件，也可以只用它切换显示器输入源。按你的工作习惯，选择适合自己的方式。
        </p>
      </div>
      <div class="about-community">
        <AppIcon name="heart" :size="16" />
        <span>由 Fantasy1316 发起，感谢每一位反馈、分享与贡献代码的朋友。</span>
      </div>
    </div>

    <UpdatePanel :current-version="version" @open-releases="open($event, 'releases')" />

    <nav class="about-links" aria-label="项目与帮助">
      <a
        v-for="link in links"
        :key="link.id"
        class="about-link"
        :href="projectLinks[link.id]"
        target="_blank"
        rel="noopener noreferrer"
        @click="open($event, link.id)"
      >
        <span class="about-link-icon"><AppIcon :name="link.icon" :size="21" /></span>
        <span class="about-link-text">
          <strong>{{ link.title }}</strong>
          <small>{{ link.detail }}</small>
        </span>
        <AppIcon class="about-external" name="external" :size="15" />
      </a>
    </nav>

    <div v-if="error" class="about-link-error" role="alert">
      <AppIcon name="alert" :size="18" />
      <div>
        <p>{{ error }}</p>
        <span class="copyable-text">{{ projectLinks[failedLink] }}</span>
      </div>
    </div>

    <div class="panel about-open-source">
      <div class="about-section-heading">
        <span class="section-icon"><AppIcon name="code" :size="20" /></span>
        <h2>开源与致谢</h2>
      </div>
      <p class="about-muted">
        项目原创代码以 MIT 许可证开放。键鼠共享基于 Deskflow，显示器控制得益于 m1ddc 与
        MonitorControl，应用由 Tauri、Rust 与 Vue 构建。感谢这些开源项目和背后的贡献者。
      </p>
      <p class="about-license-note">
        Deskflow DDC 是独立社区项目，与 Deskflow
        官方无隶属关系。内置引擎及其他第三方组件遵循各自许可证。
      </p>
      <div class="about-license-links">
        <a
          class="text-button"
          :href="projectLinks.license"
          target="_blank"
          rel="noopener noreferrer"
          @click="open($event, 'license')"
        >
          MIT 许可证<AppIcon name="external" :size="12" />
        </a>
        <a
          class="text-button"
          :href="projectLinks.notices"
          target="_blank"
          rel="noopener noreferrer"
          @click="open($event, 'notices')"
        >
          第三方开源声明<AppIcon name="external" :size="12" />
        </a>
      </div>
    </div>
    <p class="about-copyright">© 2026 Deskflow DDC contributors</p>
  </section>
</template>

<style scoped>
.about-intro {
  padding: 30px;
  background: linear-gradient(125deg, #fff 45%, #f1f6ef);
}
.about-identity {
  display: flex;
  align-items: center;
  gap: 20px;
}
.about-logo {
  flex-shrink: 0;
  border-radius: 18px;
}
.about-name {
  font-size: 26px;
  letter-spacing: -0.8px;
}
.about-tagline {
  margin-top: 4px;
  color: #63776b;
  font-size: 13px;
}
.about-badges {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  margin-top: 10px;
}
.about-badge {
  padding: 3px 9px;
  border: 1px solid #dfe8dc;
  border-radius: 6px;
  background: #f3f7f0;
  color: var(--green);
  font-size: 11px;
  line-height: 1.5;
}
.about-story {
  margin-top: 25px;
  color: #53655b;
  font-size: 13px;
}
.about-story p + p {
  margin-top: 10px;
}
.about-community {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-top: 22px;
  padding-top: 18px;
  border-top: 1px solid var(--line);
  color: #677b6c;
  font-size: 12px;
  line-height: 1.7;
}
.about-community svg {
  margin-top: 2px;
  flex-shrink: 0;
}
.about-links {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 13px;
  margin: 20px 0;
}
.about-link {
  display: flex;
  align-items: center;
  gap: 14px;
  min-width: 0;
  padding: 21px;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: #fff;
  color: var(--ink);
  text-decoration: none;
  cursor: pointer;
  transition:
    background 0.15s,
    border-color 0.15s;
}
.about-link:hover {
  border-color: #b5c9b6;
  background: #f7faf5;
}
.about-page a:focus-visible {
  outline: 3px solid #bdd2c2;
  outline-offset: 3px;
}
.about-link-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 43px;
  height: 43px;
  flex-shrink: 0;
  border-radius: 11px;
  background: var(--soft);
  color: var(--green);
}
.about-link-text {
  flex: 1;
  min-width: 0;
}
.about-link-text strong,
.about-link-text small {
  display: block;
}
.about-link-text strong {
  font-size: 14px;
  font-weight: 600;
}
.about-link-text small {
  margin-top: 6px;
  color: #718076;
  line-height: 1.6;
}
.about-external {
  flex-shrink: 0;
  color: #8b9a90;
}
.about-open-source {
  padding: 25px;
}
.about-section-heading {
  display: flex;
  align-items: center;
  gap: 11px;
  margin-bottom: 13px;
}
.about-muted {
  color: #5f7266;
  font-size: 13px;
}
.about-license-note {
  margin-top: 11px;
  color: #718076;
  font-size: 12px;
}
.about-license-links {
  display: flex;
  flex-wrap: wrap;
  gap: 10px 24px;
  margin-top: 16px;
}
.about-license-links a {
  text-decoration: none;
  cursor: pointer;
}
.about-link-error {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  margin-bottom: 20px;
  padding: 14px;
  border: 1px solid #ecdcc2;
  border-radius: 10px;
  background: #fff9ed;
  color: #8b6633;
  font-size: 12px;
  overflow-wrap: anywhere;
}
.about-link-error svg {
  margin-top: 2px;
  flex-shrink: 0;
}
.about-copyright {
  margin-top: 22px;
  color: #7c8a80;
  text-align: center;
  font-size: 11px;
}
@media (max-width: 960px) {
  .about-link {
    gap: 10px;
    padding: 17px;
  }
}
@media (max-width: 600px) {
  .about-links {
    grid-template-columns: 1fr;
  }
  .about-intro,
  .about-open-source {
    padding: 22px;
  }
  .about-identity {
    gap: 14px;
  }
  .about-logo {
    width: 60px;
    height: 60px;
    border-radius: 14px;
  }
  .about-name {
    font-size: 22px;
  }
}
</style>
