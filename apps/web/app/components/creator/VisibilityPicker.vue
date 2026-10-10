<script setup lang="ts">
// 可見性選擇 + 密碼 + 指定好友勾選（new / edit 共用）
defineProps<{ hasPassword?: boolean }>()
const visibility = defineModel<string>('visibility', { required: true })
const password = defineModel<string>('password', { default: '' })
const users = defineModel<string[]>('users', { default: () => [] })
const { api } = useApi()
const friends = ref<any[] | null>(null), fErr = ref('')
watch(visibility, async v => {
  if (v !== 'selected' || friends.value) return
  try { friends.value = (await api('/friends')).friends ?? [] } catch (e) { fErr.value = errMsg(e) }
}, { immediate: true })
function toggle(id: string, on: boolean) { users.value = on ? [...users.value, id] : users.value.filter(x => x !== id) }
</script>
<template>
  <fieldset class="c-field c-vis">
    <span>誰可以看這份清單</span>
    <label v-for="o in VIS_OPTIONS" :key="o.v" class="c-switch">
      <input v-model="visibility" type="radio" name="visibility" :value="o.v">
      <span>{{ o.label }}<br><small class="c-mute">{{ o.desc }}</small></span>
    </label>
  </fieldset>
  <label v-if="visibility === 'password'" class="c-field">
    <span>清單密碼（8–64 字）</span>
    <input v-model="password" type="password" minlength="8" maxlength="64" autocomplete="new-password" :required="!hasPassword" :placeholder="hasPassword ? '已設定密碼，留空則不變更' : ''">
  </label>
  <div v-if="visibility === 'selected'" class="c-field">
    <span>選擇可以看的好友（已選 {{ users.length }} 位）</span>
    <p v-if="fErr" class="c-err" role="alert">{{ fErr }}</p>
    <p v-else-if="!friends" class="c-mute" role="status">載入好友中…</p>
    <p v-else-if="!friends.length" class="c-mute">還沒有好友。先到 <NuxtLink to="/friends">好友</NuxtLink> 邀請朋友，再回來勾選。</p>
    <label v-for="f in friends" :key="f.id" class="c-switch">
      <input type="checkbox" :checked="users.includes(f.id)" @change="toggle(f.id, ($event.target as HTMLInputElement).checked)">
      <span>{{ f.display_name }}<small v-if="f.handle" class="c-mute">　@{{ f.handle }}</small></span>
    </label>
  </div>
</template>
<style scoped>
.c-vis{border:0;padding:0;margin:0 0 16px}
</style>
